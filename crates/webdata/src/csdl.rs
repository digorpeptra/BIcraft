//! An OData service's `$metadata` (CSDL XML, OData v4 *CSDL XML Representation*; the v2/v3
//! `edmx` documents have the same elements): its entity sets and the structural properties of
//! their entity types, base types first. Properties whose type has no M column here (complex
//! types, collections, streams, geography, durations) are left out, as are navigation
//! properties (docs/formats/odata.md).

use std::collections::HashMap;

use bicraft_data::connect::RemoteType;

/// Most XML nodes a document may have.
const MAX_NODES: u32 = 4_000_000;
/// Deepest base-type chain followed.
const MAX_BASES: usize = 32;

/// A structural property with an M column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    /// The EDM type name (`Edm.Int32`, or `Edm.String` for an enumeration).
    pub edm: String,
    pub ty: RemoteType,
}

/// An entity set and the columns of its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitySet {
    pub name: String,
    pub columns: Vec<Property>,
}

/// What `$metadata` says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub sets: Vec<EntitySet>,
}

impl Metadata {
    pub fn set(&self, name: &str) -> Option<&EntitySet> {
        self.sets.iter().find(|s| s.name == name)
    }
}

/// The M type of an EDM primitive type (`None`: no column).
pub fn edm_type(name: &str) -> Option<RemoteType> {
    Some(match name {
        "Edm.String" | "Edm.Guid" => RemoteType::Text,
        "Edm.Boolean" => RemoteType::Logical,
        "Edm.SByte" => RemoteType::Int8,
        "Edm.Byte" | "Edm.Int16" => RemoteType::Int16,
        "Edm.Int32" => RemoteType::Int32,
        "Edm.Int64" => RemoteType::Int64,
        "Edm.Single" => RemoteType::Single,
        "Edm.Double" => RemoteType::Double,
        "Edm.Decimal" => RemoteType::Decimal,
        "Edm.Date" => RemoteType::Date,
        // OData v2/v3: a date and time without offset
        "Edm.DateTime" => RemoteType::DateTime,
        "Edm.DateTimeOffset" => RemoteType::DateTimeZone,
        "Edm.TimeOfDay" => RemoteType::Time,
        "Edm.Binary" => RemoteType::Binary,
        _ => return None,
    })
}

struct Entity {
    base: Option<String>,
    props: Vec<(String, String)>,
}

fn attr<'a>(n: &roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attribute(name)
}

/// Deepest element nesting a document may have (the XML parser recurses on nesting; CSDL is a
/// handful of levels deep).
const MAX_DEPTH: usize = 256;

/// Whether the elements of `xml` nest at most [`MAX_DEPTH`] deep, counted from its tags (an
/// overestimate when a comment or an attribute value holds `<`: never an underestimate).
fn shallow(xml: &str) -> bool {
    let b = xml.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while let Some(off) = b.get(i..).and_then(|r| r.iter().position(|&c| c == b'<')) {
        let at = i + off;
        match b.get(at + 1) {
            Some(b'/') => depth = depth.saturating_sub(1),
            Some(b'!' | b'?') | None => {}
            Some(_) => {
                let end = b
                    .get(at..)
                    .and_then(|r| r.iter().position(|&c| c == b'>'))
                    .map(|e| at + e);
                let empty = end.is_some_and(|e| e > 0 && b.get(e - 1) == Some(&b'/'));
                if !empty {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return false;
                    }
                }
            }
        }
        i = at + 1;
    }
    true
}

/// Parses a `$metadata` document.
pub fn parse(xml: &str) -> Result<Metadata, String> {
    if !shallow(xml) {
        return Err(format!(
            "the OData metadata nests elements more than {MAX_DEPTH} deep"
        ));
    }
    let opts = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_NODES,
        ..roxmltree::ParsingOptions::default()
    };
    let doc = roxmltree::Document::parse_with_options(xml, opts)
        .map_err(|e| format!("the OData metadata is not XML: {e}"))?;
    // qualified names: Namespace.Name, and the same through the schema's Alias
    let mut entities: HashMap<String, Entity> = HashMap::new();
    let mut enums: Vec<String> = Vec::new();
    let mut defs: HashMap<String, String> = HashMap::new();
    let mut aliases: HashMap<String, String> = HashMap::new();
    let mut containers = Vec::new();
    for schema in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "Schema")
    {
        let Some(ns) = attr(&schema, "Namespace") else {
            continue;
        };
        if let Some(a) = attr(&schema, "Alias") {
            aliases.insert(a.to_string(), ns.to_string());
        }
        for child in schema.children().filter(roxmltree::Node::is_element) {
            let Some(name) = attr(&child, "Name") else {
                continue;
            };
            let q = format!("{ns}.{name}");
            match child.tag_name().name() {
                "EntityType" => {
                    let props = child
                        .children()
                        .filter(|p| p.is_element() && p.tag_name().name() == "Property")
                        .filter_map(|p| Some((attr(&p, "Name")?.to_string(), attr(&p, "Type")?.to_string())))
                        .collect();
                    entities.insert(
                        q,
                        Entity {
                            base: attr(&child, "BaseType").map(str::to_string),
                            props,
                        },
                    );
                }
                "EnumType" => enums.push(q),
                "TypeDefinition" => {
                    if let Some(u) = attr(&child, "UnderlyingType") {
                        defs.insert(q, u.to_string());
                    }
                }
                "EntityContainer" => containers.push(child),
                _ => {}
            }
        }
    }
    let resolve = |t: &str| -> String {
        match t.rsplit_once('.') {
            Some((prefix, name)) if aliases.contains_key(prefix) => {
                format!("{}.{name}", aliases.get(prefix).map_or(prefix, String::as_str))
            }
            _ => t.to_string(),
        }
    };
    let column = |name: &str, t: &str| -> Option<Property> {
        let mut t = resolve(t);
        for _ in 0..MAX_BASES {
            match defs.get(&t) {
                Some(u) => t = resolve(u),
                None => break,
            }
        }
        let (edm, ty) = if enums.contains(&t) {
            ("Edm.String".to_string(), RemoteType::Text)
        } else {
            let ty = edm_type(&t)?;
            (t, ty)
        };
        Some(Property {
            name: name.to_string(),
            edm,
            ty,
        })
    };
    let mut sets = Vec::new();
    for c in containers {
        for s in c
            .children()
            .filter(|s| s.is_element() && s.tag_name().name() == "EntitySet")
        {
            let (Some(name), Some(ty)) = (attr(&s, "Name"), attr(&s, "EntityType")) else {
                continue;
            };
            // the type chain, most derived first
            let mut chain = Vec::new();
            let mut cur = Some(resolve(ty));
            while let Some(t) = cur {
                if chain.len() >= MAX_BASES {
                    break;
                }
                let Some(e) = entities.get(&t) else { break };
                chain.push(e);
                cur = e.base.as_deref().map(resolve);
            }
            let mut columns: Vec<Property> = Vec::new();
            for e in chain.iter().rev() {
                for (n, t) in &e.props {
                    if columns.iter().any(|c| c.name == *n) {
                        continue;
                    }
                    if let Some(p) = column(n, t) {
                        columns.push(p);
                    }
                }
            }
            sets.push(EntitySet {
                name: name.to_string(),
                columns,
            });
        }
    }
    Ok(Metadata { sets })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_types_bases_aliases() {
        let xml = r#"<?xml version="1.0"?>
<edmx:Edmx Version="4.0" xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx">
 <edmx:DataServices>
  <Schema Namespace="Shop.Model" Alias="self" xmlns="http://docs.oasis-open.org/odata/ns/edm">
   <EnumType Name="Color"><Member Name="Red"/></EnumType>
   <TypeDefinition Name="Money" UnderlyingType="Edm.Decimal"/>
   <EntityType Name="Base"><Key><PropertyRef Name="ID"/></Key><Property Name="ID" Type="Edm.Int32" Nullable="false"/></EntityType>
   <EntityType Name="Product" BaseType="self.Base">
    <Property Name="Name" Type="Edm.String"/>
    <Property Name="Price" Type="self.Money"/>
    <Property Name="Color" Type="Shop.Model.Color"/>
    <Property Name="Tags" Type="Collection(Edm.String)"/>
    <Property Name="Since" Type="Edm.DateTimeOffset"/>
    <NavigationProperty Name="Category" Type="self.Category"/>
   </EntityType>
   <EntityContainer Name="Shop">
    <EntitySet Name="Products" EntityType="self.Product"/>
    <Singleton Name="Me" Type="self.Product"/>
   </EntityContainer>
  </Schema>
 </edmx:DataServices>
</edmx:Edmx>"#;
        let m = parse(xml).unwrap();
        let p = m.set("Products").unwrap();
        let cols: Vec<(&str, RemoteType)> = p.columns.iter().map(|c| (c.name.as_str(), c.ty)).collect();
        assert_eq!(
            cols,
            [
                ("ID", RemoteType::Int32),
                ("Name", RemoteType::Text),
                ("Price", RemoteType::Decimal),
                ("Color", RemoteType::Text),
                ("Since", RemoteType::DateTimeZone),
            ]
        );
        assert_eq!(m.sets.len(), 1);
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("<!DOCTYPE x [<!ENTITY e 'x'>]><x>&e;</x>").is_err());
    }
}
