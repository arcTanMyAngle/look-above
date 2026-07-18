# Base map geometry (M2 item 2.2a)

`land.geojson` and `coastline.geojson` — Natural Earth's 1:50m physical vectors, bundled here
so `render` never touches the network (ADR-002; docs/03 "Base map geometry"). Consumed by
`render`'s tessellation pipeline (item 2.2b) at startup via `include_bytes!` or a file read —
never fetched live by the app.

## Provenance

- **Source:** Natural Earth, 1:50m scale, `ne_50m_land` and `ne_50m_coastline`
  (public domain — [naturalearthdata.com](https://www.naturalearthdata.com), no attribution
  required).
- **Actual download host:** `https://naciscdn.org/naturalearth/50m/physical/*.zip`. docs/03
  points at `naturalearthdata.com/downloads/`, but that page's own direct file links 404 —
  verified, not assumed. `naciscdn.org` is Natural Earth's real CDN, linked from the same
  downloads page; confirmed live (`200`, ~450 KB per zip). See DECISION_LOG 2.2a.
- **Converted by:** `cargo run -p look-above-import --bin import-basemap`
  (`crates/import/src/import_basemap.rs`). Shapefile → GeoJSON, no simplification beyond what
  1:50m already is, coordinates rounded to 1e-4° (~11 m).

## Format

Plain `FeatureCollection`s. `land.geojson`: one `Polygon` feature per shapefile outer ring
(plus any holes immediately following it — see the import tool's doc comment for the grouping
rule). `coastline.geojson`: one `LineString` feature per shapefile part. No `properties` — the
base map carries no per-feature metadata, only geometry.

| File | Features | Points | Size |
|---|---|---|---|
| `land.geojson` | 1,421 polygons | 60,669 | ~1.2 MB |
| `coastline.geojson` | 1,429 lines | 60,416 | ~1.2 MB |

## Refreshing

Natural Earth's 1:50m physical vectors change rarely (this bundle is v4.0.0). Re-run the
import tool and re-commit both files if they ever do:

```text
cargo run -p look-above-import --bin import-basemap
```
