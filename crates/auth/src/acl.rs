//! Identity → workspace authorization.
//!
//! A list of grants, each naming a subject, the workspaces it opens and an access level. With no
//! matching grant, a workspace is closed (and answered as missing, so its existence is not
//! revealed). Readers and editors are told apart by [`Access`]: the highest level among the
//! matching grants wins (there are no deny rules). What a reader may run is declared by each engine
//! command (`bicraft_engine::Effect`, docs/auth.md "Roles").

use serde_json::Value;

use crate::{Identity, Source};

/// Who a grant applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// `*`: any authenticated identity.
    Any,
    /// `user:<subject>`: an OIDC subject claim or the proxy's user header.
    User(String),
    /// `email:<address>` (ASCII case-insensitive).
    Email(String),
    /// `group:<name>`: an OIDC groups claim value, a proxy group or a token's group.
    Group(String),
    /// `role:<name>`: a mapped OIDC role or a token's role.
    Role(String),
    /// `token:<name>`: one API token.
    Token(String),
    /// `issuer:<url>`: every identity from one OIDC issuer.
    Issuer(String),
}

impl Subject {
    pub fn parse(s: &str) -> Result<Subject, String> {
        let s = s.trim();
        if s == "*" {
            return Ok(Subject::Any);
        }
        let (kind, value) = s.split_once(':').ok_or(format!(
            "`{s}`: a subject is `*` or kind:value (user, email, group, role, token, issuer)"
        ))?;
        if value.is_empty() {
            return Err(format!("`{s}`: empty subject value"));
        }
        let v = value.to_string();
        Ok(match kind {
            "user" => Subject::User(v),
            "email" => Subject::Email(v),
            "group" => Subject::Group(v),
            "role" => Subject::Role(v),
            "token" => Subject::Token(v),
            "issuer" => Subject::Issuer(v),
            other => return Err(format!("`{s}`: unknown subject kind `{other}`")),
        })
    }

    fn matches(&self, id: &Identity) -> bool {
        match self {
            Subject::Any => true,
            Subject::User(u) => !matches!(id.source, Source::Token) && id.subject == *u,
            Subject::Email(e) => id.email.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(e)),
            Subject::Group(g) => id.groups.iter().any(|x| x == g),
            Subject::Role(r) => id.roles.iter().any(|x| x == r),
            Subject::Token(t) => matches!(id.source, Source::Token) && id.subject == *t,
            Subject::Issuer(i) => matches!(&id.source, Source::Oidc { issuer } if issuer == i),
        }
    }
}

/// Access level on a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    /// Read-only: inspect, validate, evaluate, render without writing; no edit, save, upload,
    /// deletion, refresh or network (docs/auth.md "Roles").
    Reader,
    /// Every command allowed in server mode.
    Editor,
    // room for an owner or admin level above (workspace or ACL management): the order is the
    // precedence
}

/// Which workspaces a grant opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Workspaces {
    All,
    List(Vec<String>),
}

impl Workspaces {
    fn contains(&self, ws: &str) -> bool {
        match self {
            Workspaces::All => true,
            Workspaces::List(l) => l.iter().any(|w| w == ws),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub subject: Subject,
    pub workspaces: Workspaces,
    pub access: Access,
}

fn parse_access(s: &str) -> Result<Access, String> {
    match s {
        "editor" => Ok(Access::Editor),
        "reader" => Ok(Access::Reader),
        other => Err(format!("unknown access `{other}` (reader, editor)")),
    }
}

fn parse_workspaces(list: &[&str]) -> Result<Workspaces, String> {
    if list.contains(&"*") {
        return Ok(Workspaces::All);
    }
    if list.is_empty() {
        return Err("a grant names at least one workspace (or `*`)".into());
    }
    for w in list {
        if w.is_empty()
            || w.len() > 64
            || w.starts_with('.')
            || !w
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(format!("`{w}` is not a workspace id"));
        }
    }
    Ok(Workspaces::List(list.iter().map(|w| (*w).to_string()).collect()))
}

impl Grant {
    /// `SUBJECT=WORKSPACES`, e.g. `group:bi-team=*` or `email:ana@example.com=sales,hr`. The
    /// last `=` separates (LDAP group names contain `=`).
    pub fn parse_text(s: &str) -> Result<Grant, String> {
        Self::parse_text_as(s, Access::Editor)
    }

    /// [`Grant::parse_text`] with an explicit access level (`--reader-grant`,
    /// `BICRAFT_AUTH_READER_GRANTS`).
    pub fn parse_text_as(s: &str, access: Access) -> Result<Grant, String> {
        let (subject, ws) = s.rsplit_once('=').ok_or(format!(
            "`{s}`: a grant is SUBJECT=WORKSPACES (e.g. group:bi-team=*)"
        ))?;
        let list: Vec<&str> = ws.split(',').map(str::trim).filter(|w| !w.is_empty()).collect();
        Ok(Grant {
            subject: Subject::parse(subject)?,
            workspaces: parse_workspaces(&list)?,
            access,
        })
    }

    /// `{"subject": "group:bi-team", "workspaces": "*" | ["sales"], "access": "editor"}`.
    pub fn parse_json(v: &Value) -> Result<Grant, String> {
        let o = v.as_object().ok_or("an ACL entry is an object")?;
        for k in o.keys() {
            if !matches!(k.as_str(), "subject" | "workspaces" | "access" | "comment") {
                return Err(format!("ACL entry: unknown field `{k}`"));
            }
        }
        let subject = o
            .get("subject")
            .and_then(Value::as_str)
            .ok_or("ACL entry: `subject` is required")?;
        let list: Vec<&str> = match o.get("workspaces") {
            Some(Value::String(s)) => vec![s.as_str()],
            Some(Value::Array(a)) => a
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()
                .ok_or("ACL entry: `workspaces` is `*` or a list of ids")?,
            _ => return Err("ACL entry: `workspaces` is `*` or a list of ids".into()),
        };
        let access = match o.get("access") {
            None => Access::Editor,
            Some(Value::String(a)) => parse_access(a)?,
            Some(_) => return Err("ACL entry: `access` is a string".into()),
        };
        Ok(Grant {
            subject: Subject::parse(subject)?,
            workspaces: parse_workspaces(&list)?,
            access,
        })
    }
}

/// The grants.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Acl {
    pub grants: Vec<Grant>,
}

impl Acl {
    /// The highest access `id` has on `workspace`, `None` when closed.
    pub fn access(&self, id: &Identity, workspace: &str) -> Option<Access> {
        self.grants
            .iter()
            .filter(|g| g.workspaces.contains(workspace) && g.subject.matches(id))
            .map(|g| g.access)
            .max()
    }
}
