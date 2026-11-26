# Map visuals: map, filled map, shape map, Azure Maps

Sources: Microsoft Learn, [Create Shape Map visualizations in Power BI](https://learn.microsoft.com/en-us/power-bi/visuals/desktop-shape-map)
(wells, Map settings, custom TopoJSON and GeoJSON maps, map keys, projections, colours, the
1,500 data point limit) and [Filled maps (choropleths)](https://learn.microsoft.com/en-us/power-bi/visuals/power-bi-visualization-filled-maps-choropleths); Natural Earth (public domain, [naturalearthdata.com](https://www.naturalearthdata.com/)) for the bundled outlines and the open basemap; the public report theme JSON schema vendored in `crates/visuals/schemas`
(the four types' formatting objects); the public PBIR `report.json` schemas vendored in
`crates/pbir/schemas` (the `ShapeMap` resource item type); the data roles of
[visual-roles-bb.md](visual-roles-bb.md) (black-box tests against Desktop); a black-box check of a
BIcraft-made PBIX in Power BI Desktop 2.159.1231.0 (Store) on 2026-11-23, [below](#checked-in-desktop).
The custom map reference inside the visual is BIcraft's reading of the schemas; Desktop opens it,
draws it and saves it back unchanged.

## Visual types and wells

| Visual (Desktop) | `visualType` | Wells (query roles) |
|---|---|---|
| Map (Bing) | `map` | Location `Category`, Legend `Series`, Latitude `Y`, Longitude `X`, Bubble size `Size`, Tooltips |
| Filled map | `filledMap` | Location `Category`, Legend `Series`, Latitude `Y`, Longitude `X`, Tooltips |
| Shape map | `shapeMap` | Location `Category`, Legend `Series`, Color saturation `Value`, Tooltips |
| Azure Maps | `azureMap` | Location `Category`, Latitude `Y`, Longitude `X`, Legend `Series`, Size, Tooltips, `PathID`, `PointOrder` |

`visual.add` / `visual.bind` take these roles (`crates/visuals/src/roles_bb.rs`); the Build pane's
type picker inserts them. The Format pane's cards come from the theme schema's objects:
`bubbles`, `categoryLabels`, `dataPoint`, `heatMap`, `legend`, `mapControls`, `mapStyles` (map);
`dataPoint`, `defaultColors`, `legend`, `shape`, `zoom` (shape map, `shape` titled Map settings as
on Learn); `visual.format` writes any of them.

## Custom shape maps

Learn: Format › Map settings › Map type › Custom map › *Add a map type* loads a `.topojson` or
`.geojson` file; *Type* › URL reads a map from a link instead (`shape.datasourceType` `url`,
`shape.mapUrl`, theme schema). Region keys are a map's ids and property values (Learn's key
tables list ID, ISO, name and postal columns side by side).

`shape_map.set_map {page, visual, path | data, name?, remove?}` writes:

- the file as `StaticResources/RegisteredResources/<name><16 digits>.json` (or `.topojson`), an
  item `{name, path, type: "ShapeMap"}` of the `RegisteredResources` package of `report.json`
  (`ShapeMap` is an item type of the public PBIR schema, every version);
- in the visual, `objects.shape[0].properties`:
  `datasourceType: 'file_upload'` (the theme schema's value besides `url`) and
  `map: {geoJson: {type: 'custom', name: '<file name>', content: ResourcePackageItem}}`, the
  `ResourcePackageItem` as image visuals name their pictures. This shape is BIcraft's reading;
  Desktop 2.159.1231.0 reads it and saves it back unchanged.

A replaced or removed map drops its item and file when nothing else names it.

**Legacy layout.** The legacy `Report/Layout` stores resource item types as numbers; only 100
(Image), 201 (CustomTheme) and 202 (BaseTheme) are observed ([legacy-layout-bb.md](legacy-layout-bb.md)).
Saving a custom shape map to a legacy-layout PBIX is refused with an error naming the item type,
never written with a guessed code; PBIP and a PBIX with PBIR parts (what `report.new` writes) keep it.

### Reading the map file

Untrusted input: at most 16 MB, 10,000 shapes, 2 million decoded points, arc indices checked,
geometry collections at most 8 deep.

- **TopoJSON** (`type: Topology`): `arcs` delta-decoded and scaled when `transform` is present;
  a negative arc index `~i` is arc `i` reversed; every object's `Polygon` and `MultiPolygon`
  geometries become shapes (points and lines have no area and are skipped).
- **GeoJSON**: a `FeatureCollection`, a `Feature` or a polygon geometry; `GeometryCollection`
  parts join one shape.
- A shape's keys: its `id`, then its property values (text and numbers), at most 16.

The snapshot (`shape_map` of the visual) carries `{source: custom|url|builtin, name, projection,
auto_zoom (Zoom › Auto zoom), default_color, border_color, border_width, show_blank, shapes: [{keys, rings: [[x, y, …]]}],
simplified, error}`; rings are thinned evenly to 40,000 points in all.

### BIcraft's built-in map: World countries (Natural Earth)

Desktop's built-in shape maps are Microsoft's map data and are not bundled. Instead,
`shape_map.set_map {builtin: "world_countries"}` (Format › Map settings › Built-in map › World
countries (Natural Earth)) **writes Natural Earth's countries into the report as an ordinary
custom map**: a TopoJSON file `World countries (Natural Earth).topojson` stored as a `ShapeMap`
item and named by `shape.map` exactly as above, so Desktop sees a custom map like any other
(Learn: custom shape maps are TopoJSON); there is no BIcraft-only reference in the file. The
topology (under 400 KB, test-checked) has one object `countries`, 176 countries (Antarctica left out: it would
fill the bottom of a Mercator map), each a Polygon or MultiPolygon with `id` = ISO 3166-1
alpha-3 (else the name) and properties `name`, `iso_a2`, `iso_a3`, `name_long`, `admin`,
`formal_en`, `name_fr` (those Natural Earth has), coordinates quantized to 0.01° through
`transform`. Locations match any of these keys. Desktop 2.159.1231.0 opens and draws it ([below](#checked-in-desktop)).

### Bundled country outlines

`crates/render/geo/ne_110m_admin_0_countries.bin` (41 KB) holds Natural Earth's 1:110m Admin 0
countries (public domain; [nvkelso/natural-earth-vector](https://github.com/nvkelso/natural-earth-vector)
`geojson/ne_110m_admin_0_countries.geojson` at `ca96624a56bd078437bca8184e78163e5039ad19`,
SHA-256 in `crates/render/geo/SOURCE.txt`), encoded by `cargo xtask geo-bundle SRC`
(xtask/src/geo_bundle.rs, which documents the format): 177 countries, 287 polygons, 10,306
points quantized to 0.01°, the ISO codes (`ISO_A2_EH` / `ISO_A3_EH`, which fill the `-99` that
`ISO_A2` holds for France and Norway), the names `NAME`, `NAME_LONG`, `ADMIN`, `FORMAL_EN`,
`NAME_FR` and the label point `LABEL_X` / `LABEL_Y`. `crates/render/src/geo.rs` decodes it
(bounded reads, a damaged asset gives no countries) and matches a location without case,
accents or repeated spaces.

### The open basemap's detailed layers (2026-11-24)

By the maintainers' decision of 2026-11-24, BIcraft draws maps on a free, open basemap instead
of depending on Bing. `crates/render/geo/ne_50m_basemap.bin` (329,555 bytes; provenance and
SHA-256 in `crates/render/geo/SOURCE-50m.txt`) holds four Natural Earth 1:50m layers of the same
commit, each source SHA-256 checked, encoded by `cargo xtask geo-bundle --basemap DIR` (format
documented in xtask/src/geo_bundle.rs), coordinates quantized to 0.01°:

| Layer | Source (`geojson/`) | Kept | Count |
|---|---|---|---|
| Countries | `ne_50m_admin_0_countries.geojson` | `ISO_A3_EH` / `ISO_A3` / `ADM0_A3`, `ADMIN`, polygons | 242 |
| Lakes | `ne_50m_lakes.geojson` | polygons | 411 |
| Admin-1 boundaries | `ne_50m_admin_1_states_provinces_lines.geojson` (Australia, Brazil, Canada, China, India, Indonesia, Russia, South Africa, United States) | lines | 591 |
| Populated places | `ne_50m_populated_places_simple.geojson` | `name`, `nameascii`, `adm0_a3`, `adm1name`, position, `scalerank`, `adm0cap`, `pop_max` | 1,251 |

132,318 points in all. The binary and the browser app's wasm grow by about the file's size
(estimated: `include_bytes!`; the 1:110m file stays, 41 KB). `crates/render/src/geo.rs` decodes
it once (bounded reads; a damaged asset gives empty layers, test-checked), maps each 1:50m
country to its 1:110m country (ISO alpha-3, else name) so a filled country is filled at both
scales, and places cities (`find_city`, below).

**Online tiles: not done.** An opt-in tile server (an OpenStreetMap-style URL template) was
considered and left out: the renderer is synchronous and below the layers that may use the
network (AGENTS.md §4), the browser app cannot fetch tiles synchronously, PDF export would
embed rasters of a third-party server, and OpenStreetMap's tile usage policy (attribution,
an identifying User-Agent, caching, no bulk or offline download, heavy use not allowed on its
servers) would need a cache and a per-server opt-in setting to be honoured. The vector basemap
covers the need offline, in every output, without keys or network.

## Drawing offline (`crates/render/src/maps.rs`)

No tiles; the base map is the bundled Natural Earth vector data. Drawing only: nothing in this
section writes to the file.

- **Map, filled map, Azure Maps**: a Web Mercator base map (the sea a tint of the first data
  colour over the theme's band colour, land near the background, borders between, parallels and
  meridians on the sea; all from the theme's text and background, light and dark), marked
  "Made with Natural Earth" (*Réalisé avec Natural Earth* in French), the credit Natural Earth
  suggests (its data is public domain; no attribution is required).
  - **Level of detail**, by the longitudes the view spans: 90° or more, the 1:110m countries;
    under 90°, the 1:50m countries and lakes (lakes in the sea colour); under 70°, also the
    admin-1 boundaries (dashed, between land and border colours) and city names: a dot (larger
    for national capitals) and the name on a veil of the land colour, the larger places first
    (`scalerank`, then population), about one per 16,000 square pixels, never over another name
    or a data bubble.
  - With Latitude and Longitude: bubbles, the view 15 % around the points (at least 6° across),
    sized by the square root of Size, coloured by Legend (legend at the top).
  - With Location only, **offline geocoding of countries**: a location holding a country's name
    (English, Natural Earth's French name, long or formal name) or ISO alpha-2 / alpha-3 code
    puts its bubble at the country's label point; the view is 5 % around the main land of the
    countries named (the widest polygon: mainland France without French Guiana).
  - **Offline geocoding of major cities** (2026-11-24, BIcraft's own rule): a location that is
    not a country but the name (or ASCII name) of one of Natural Earth's 1,251 populated places
    puts its bubble there; among places of one name the most populous wins (`Portland` is
    Oregon's), unless the text adds `, ` and the country (name or ISO code) or the state or
    province (`Portland, Victoria`, `Córdoba, Espagne`). Cities alone: the view is 15 % around
    them, at least 6° across; with countries, 5 % around both.
  - **Filled map** whose locations are countries: the countries filled with their Legend colour
    (else the first data colour), each a polygon hit; Learn's colour saturation by conditional
    formatting (Fill colours fx) is not applied. A filled map whose locations are not countries
    draws bubbles as above.
  - Locations that are neither countries nor bundled cities are counted in the render note (they
    need geocoding).
- **Place names other than countries and major cities only**: a placeholder saying that offline
  only countries and major cities are placed and other places need the online map service (Bing
  or Azure Maps), status `placeholder`. With no rows at all, the world's outlines.
- **Shape map**: the custom map's areas in its projection (`shape.projectionEnum`: Mercator by
  default, equirectangular, orthographic around the map's centre, Albers USA as one conic without
  insets); a map whose coordinates are not longitudes and latitudes (floor plans) is drawn as it
  is. Locations match a shape's keys without case. Colours: Legend → the theme's data colours and
  a legend; otherwise every matched area takes the first data colour: a Color saturation field
  alone does not shade the areas, as Desktop draws it (below; the shading set in Colors › Color
  saturation is not written or drawn). Unmatched areas take
  `defaultColors.defaultColor` (else a light grey) or are hidden when `defaultShow` is false;
  borders take `borderColor` / `borderThickness` (else 1 px of the background).
  Zoom › Auto zoom (`zoom.autoZoom`, theme schema: "zoom in on shapes with available data") fits
  the matched areas; off or unset, the whole map (as Desktop draws a map without the property). Without a
  custom map (Desktop's built-in maps are Microsoft's data) and for maps from a URL: placeholders;
  the built-in placeholder points to World countries (Natural Earth).

Each bubble and matched area is a hit: a click sends `view.interact select` with its location
(and legend) values, which cross-filters the page as other visuals do; the tooltip lists the
row's fields. Unmatched areas show their first key.

## Checked in Desktop

Power BI Desktop 2.159.1231.0 (Store edition), 2026-11-23, French UI. A PBIX made with the CLI
over a synthetic table of six countries (name, a city code, latitude, longitude, a value): a shape
map with `shape_map.set_map {builtin: "world_countries"}`, Location = country and Color
saturation = Sum of the value; a filled map with Location = country; a map with Location, Latitude,
Longitude (averages) and Size. Desktop opened the file, the pages were shown and captured
(screenshots kept outside the repository), Save was clicked and the PBIX read back.

- **Shape map: accepted and drawn.** Desktop draws the World countries (Natural Earth) TopoJSON as
  written: the 176 countries in Mercator, the six matched countries filled, the others grey
  (`#C8C8C8`, white borders), the whole world shown. Its Format pane, Map settings: Type "file
  upload" (*Chargement de fichiers*), the map named "World countries (Natural Earth).topojson",
  Projection Mercator; Legend off. So `datasourceType` `file_upload` and `map.geoJson` {`type`
  `custom`, `name`, `content` as a `ResourcePackageItem`} are read as BIcraft writes them.
- **Saved back unchanged.** Desktop's save keeps the `shape` object as BIcraft wrote it and the
  `ShapeMap` item and TopoJSON file byte for byte; it adds an empty `dataPoint` object and one
  visual-level filter per field.
- **Color saturation alone does not shade.** All six matched countries take one colour
  (`#01B8AA`, the first data colour of the report's default theme) though their values run from
  10 to 50; the Format pane's Colors › Color saturation card shows a minimum and maximum colour
  (`#FD625E` to `#01B8AA`, both Automatic) that the drawing does not use. BIcraft now draws the
  same (it drew a gradient before).
- **Map and filled map: not drawn here.** Both show Desktop's message that map and filled map
  visuals are turned off (File › Options and settings › Options › Global › Security). The option
  was left unchanged (it lets Desktop send locations to Bing), so bubbles and filled countries
  were not compared.

## Unknowns

- What Desktop writes when Colors › Color saturation is set, and how it then shades; the legacy
  item type code of `ShapeMap`; Auto zoom's effect in Desktop (unset, the whole map is shown).
- Map and filled map drawing in Desktop (switched off in the security options of the Desktop
  used).
- Which TopoJSON object Desktop reads when a topology has several (BIcraft reads them all).
