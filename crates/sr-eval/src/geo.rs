//! Map cameras, shared by the renderer (which draws `<map>` assets) and by
//! expressions (`geo()` and `geoVisible()` place any layer on a map).
//!
//! A camera is fitted once from the map's unanimated attributes and its fit
//! data; the view at a time then follows the animated centre, zoom and
//! rotation, and the `<flyTo>` moves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::SystemTime;

use sr_geo::data::{self, Feature, Format, Geometry};
use sr_geo::mvt;
use sr_geo::pmtiles::Archive;
use sr_geo::style::Style;
use sr_geo::view::{Fly, Kind, Map, View};
use sr_model::model as m;

use crate::curve::{self, KeyParams};
use crate::Program;

/// A fitted map: the projection setup at zoom 0 and the centre it looks at by default.
#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub map: Map,
    pub center: [f64; 2],
}

#[cfg(test)]
thread_local! {
    /// Times a camera's fit signature was built (the fit files stat'ed).
    static FITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A file's size and modification time: what was read from it is current while they match.
type Stamp = (u64, Option<SystemTime>);

fn stamp(path: &Path) -> Result<Stamp, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((meta.len(), meta.modified().ok()))
}

/// A process cache of what was read from files: one reading per key, replaced when its files
/// change rather than kept beside the new one, so a long preview session holds a reading per
/// file it names however often the files are rewritten.
type Readings<K, S, V> = OnceLock<Mutex<HashMap<K, (S, V)>>>;

/// Keys a [`Readings`] holds before it is emptied (as many maps, files or setups in one session).
const READINGS: usize = 256;

/// The reading under `key` while its files still carry `stamp`, else `read()` in its place.
fn reading<K: std::hash::Hash + Eq, S: PartialEq, V: Clone>(
    cache: &Readings<K, S, V>,
    key: K,
    stamp: S,
    read: impl FnOnce() -> Result<V, String>,
) -> Result<V, String> {
    let cache = cache.get_or_init(Default::default);
    let lock = || cache.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((s, v)) = lock().get(&key) {
        if *s == stamp {
            return Ok(v.clone());
        }
    }
    match read() {
        Ok(v) => {
            let mut c = lock();
            if c.len() >= READINGS && !c.contains_key(&key) {
                c.clear();
            }
            c.insert(key, (stamp, v.clone()));
            Ok(v)
        }
        Err(e) => {
            lock().remove(&key);
            Err(e)
        }
    }
}

type FeatureKey = (PathBuf, String, Option<String>);
static FEATURES: Readings<FeatureKey, Stamp, Result<Arc<Vec<Feature>>, String>> = OnceLock::new();

/// Features of a geo file, read once per process while its size and modification time hold.
pub fn load(path: &Path, format: &str, object: Option<&str>) -> Result<Arc<Vec<Feature>>, String> {
    let key = (path.to_path_buf(), format.to_string(), object.map(str::to_string));
    reading(&FEATURES, key, stamp(path)?, || Ok(data::load(path, Format::parse(format), object).map(Arc::new)))?
}

/// The asset `id` of document `doc` (0 the main document, then the includes): a map, and the geo and tiles assets
/// its layers name, are those of the document that declares the map.
fn doc_asset<'p>(p: &'p Program, doc: u16, id: &str) -> Option<&'p m::AssetsChild> {
    p.doc_scene(doc)?.assets.as_ref()?.children.iter().find(|c| c.id() == Some(id))
}

/// The geo asset `id` of document `doc` and its file.
pub fn geo_asset<'p>(p: &'p Program, doc: u16, id: &str) -> Result<(&'p m::GeoAsset, PathBuf), String> {
    let a = doc_asset(p, doc, id).ok_or_else(|| format!("no geo asset {id:?}"))?;
    let m::AssetsChild::Geo(g) = a else { return Err(format!("{id} is not a geo asset")) };
    let base = p.doc_base(doc);
    match sr_model::assets::resolve(&g.src, &base) {
        sr_model::assets::Resolved::Local(path) => Ok((g, path)),
        sr_model::assets::Resolved::Remote(u) => Err(format!("{u}: remote geo data is not fetched while rendering")),
    }
}

/// The features of the geo asset `id` of document `doc`.
pub fn features(p: &Program, doc: u16, id: &str) -> Result<Arc<Vec<Feature>>, String> {
    let (g, path) = geo_asset(p, doc, id)?;
    load(&path, &g.format.to_string(), g.object.as_deref())
}

/// The map asset `id` of document `doc`.
pub fn map_asset<'p>(p: &'p Program, doc: u16, id: &str) -> Option<&'p m::MapAsset> {
    match doc_asset(p, doc, id)? {
        m::AssetsChild::Map(mp) => Some(mp),
        _ => None,
    }
}

/// The fitted camera of a map asset of document `doc`, fitted once per program.
pub fn camera(p: &Program, doc: u16, mp: &m::MapAsset) -> Result<Arc<Camera>, String> {
    let memo = || p.map_cameras.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cam) = memo().get(&doc).and_then(|m| m.get(mp.id.as_str())) {
        return Ok(cam.clone());
    }
    let cam = fit(p, doc, mp)?;
    memo().entry(doc).or_default().insert(mp.id.clone(), cam.clone());
    Ok(cam)
}

static CAMERAS: Readings<String, Vec<Stamp>, Arc<Camera>> = OnceLock::new();

/// The camera of a map asset, cached per process by its setup while its fit files keep their size and modification time.
fn fit(p: &Program, doc: u16, mp: &m::MapAsset) -> Result<Arc<Camera>, String> {
    let kind = Kind::parse(&mp.projection.to_string()).ok_or("unknown projection")?;
    let parallels = mp.parallels.as_ref().filter(|p| p.len() >= 2).map(|p| [p[0], p[1]]);
    let fit_ids = mp.fit.clone().unwrap_or_default();
    let mut sig = format!(
        "{}|{parallels:?}|{}x{}|{:?}|{:?}|{}|{}",
        mp.projection,
        mp.width,
        mp.height,
        mp.center_lon,
        mp.center_lat,
        mp.fit_padding.get(),
        mp.precision.get()
    );
    #[cfg(test)]
    FITS.with(|c| c.set(c.get() + 1));
    let mut stamps = Vec::with_capacity(fit_ids.len());
    for id in &fit_ids {
        let (g, path) = geo_asset(p, doc, id)?;
        stamps.push(stamp(&path)?);
        sig.push_str(&format!("|{}|{:?}", path.display(), g.object));
    }
    reading(&CAMERAS, sig, stamps, || fit_fresh(p, doc, mp, kind, parallels, &fit_ids))
}

fn fit_fresh(
    p: &Program,
    doc: u16,
    mp: &m::MapAsset,
    kind: Kind,
    parallels: Option<[f64; 2]>,
    fit_ids: &[String],
) -> Result<Arc<Camera>, String> {
    let mut geoms: Vec<Geometry> = Vec::new();
    for id in fit_ids {
        geoms.extend(features(p, doc, id)?.iter().map(|f| f.geometry.clone()));
    }
    let refs: Vec<&Geometry> = geoms.iter().collect();
    let size = [mp.width as f64, mp.height as f64];
    let (clon, clat) = (mp.center_lon.map(|v| v.get()), mp.center_lat.map(|v| v.get()));
    let center = clon.zip(clat).map(|(a, b)| [a, b]);
    let (mut map, mut c) = Map::new(kind, parallels, size, &refs, mp.fit_padding.get(), center);
    if let Some(lon) = clon {
        c[0] = lon;
    }
    if let Some(lat) = clat {
        c[1] = lat;
    }
    map.precision = mp.precision.get();
    // room round the frame for strokes, pins and labels reaching in from outside
    map.margin = 64.0;
    Ok(Arc::new(Camera { map, center: c }))
}

/// The view of a map at composition time `t`. `animated` gives the evaluated value of an
/// animated map attribute (`centerLon`, `centerLat`, `zoom`, `rotation`) when it has one.
pub fn view(cam: &Camera, mp: &m::MapAsset, animated: &dyn Fn(&str) -> Option<f64>, t: f64) -> View {
    let base = View {
        lon: animated("centerLon").or(mp.center_lon.map(|v| v.get())).unwrap_or(cam.center[0]),
        lat: animated("centerLat").or(mp.center_lat.map(|v| v.get())).unwrap_or(cam.center[1]),
        zoom: animated("zoom").unwrap_or(mp.zoom),
        rotation: animated("rotation").unwrap_or(mp.rotation),
    };
    let rest = View {
        lon: mp.center_lon.map(|v| v.get()).unwrap_or(cam.center[0]),
        lat: mp.center_lat.map(|v| v.get()).unwrap_or(cam.center[1]),
        zoom: mp.zoom,
        rotation: base.rotation,
    };
    let mut flies = Vec::new();
    let mut eases = Vec::new();
    for c in &mp.children {
        if let m::MapAssetChild::FlyTo(f) = c {
            flies.push(Fly {
                begin: f.begin,
                duration: f.duration.map(|d| d.get()),
                lon: f.lon.get(),
                lat: f.lat.get(),
                zoom: f.zoom,
                rho: f.rho.get(),
            });
            eases.push((f.begin, curve::resolve(f.easing, &KeyParams::default())));
        }
    }
    // each move eases by its own curve: the latest to have begun
    let active = eases.iter().filter(|(b, _)| *b <= t).max_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, e)| *e);
    let ease = |u: f64| active.map_or(u, |e| e.apply(u));
    cam.map.view_at(t, base, rest, &flies, &ease)
}

/// Where a longitude/latitude lands on a map (map pixels) and whether it is visible there
/// (on the near side of a globe and inside the frame).
pub fn locate(cam: &Camera, v: &View, lon: f64, lat: f64) -> ([f64; 2], bool) {
    let proj = cam.map.projection(v);
    let q = proj.point_unclipped(lon, lat);
    let inside = q[0] >= 0.0 && q[1] >= 0.0 && q[0] <= cam.map.size[0] && q[1] <= cam.map.size[1];
    (q, proj.visible(lon, lat) && inside)
}

/// The tiles asset `id` of document `doc` and the archive it reads: `src`, or the `cache` that
/// `scene-render resolve` fills from `url`.
pub fn tiles_asset<'p>(p: &'p Program, doc: u16, id: &str) -> Result<(&'p m::TilesAsset, PathBuf), String> {
    let a = doc_asset(p, doc, id).ok_or_else(|| format!("no tiles asset {id:?}"))?;
    let m::AssetsChild::Tiles(t) = a else { return Err(format!("{id} is not a tiles asset")) };
    let src =
        t.src.as_deref().or(t.cache.as_deref()).ok_or_else(|| {
            format!("tiles {id}: needs @src, or @url with a @cache that `scene-render resolve` fills")
        })?;
    let base = p.doc_base(doc);
    match sr_model::assets::resolve(src, &base) {
        sr_model::assets::Resolved::Local(path) => Ok((t, path)),
        sr_model::assets::Resolved::Remote(u) => Err(format!("{u}: tiles are not fetched while rendering")),
    }
}

static ARCHIVES: Readings<PathBuf, Stamp, Arc<Archive>> = OnceLock::new();

/// An open tile archive (per process, while the file keeps its size and modification time).
pub fn archive(path: &Path) -> Result<Arc<Archive>, String> {
    reading(&ARCHIVES, path.to_path_buf(), stamp(path)?, || Archive::open(path).map(Arc::new))
}

/// Tiles key on their archive's address; each entry holds the archive weakly, which keeps that
/// address from being reused by a newer archive while the entry lives.
type TileCache = HashMap<(usize, u8, u32, u32), (Weak<Archive>, Option<Arc<TileData>>)>;

/// A tile's contents: vector layers, or the encoded image of a raster tile.
pub enum TileData {
    Vector(Vec<mvt::Layer>),
    Raster(Arc<Vec<u8>>),
}

/// Tile z/x/y of an archive, decoded (a bounded cache of recent tiles).
pub fn tile(a: &Arc<Archive>, z: u8, x: u32, y: u32) -> Result<Option<Arc<TileData>>, String> {
    static CACHE: OnceLock<Mutex<TileCache>> = OnceLock::new();
    let key = (Arc::as_ptr(a) as usize, z, x, y);
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, t)) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        return Ok(t.clone());
    }
    let t = match a.tile(z, x, y)? {
        None => None,
        Some(b) if a.header.tile_type.raster() => Some(Arc::new(TileData::Raster(Arc::new(b)))),
        Some(b) => Some(Arc::new(TileData::Vector(mvt::decode(&b)?))),
    };
    let mut c = cache.lock().unwrap_or_else(|p| p.into_inner());
    if c.len() > 1024 {
        c.clear();
    }
    c.insert(key, (Arc::downgrade(a), t.clone()));
    Ok(t)
}

/// Basemap styles: built-in ones by name (no stamp), style files by path.
static STYLES: Readings<PathBuf, Option<Stamp>, Arc<Style>> = OnceLock::new();

/// A basemap style: a built-in name (`protomaps-light` by default, `protomaps-dark`) or a style
/// file relative to document `doc`, the one that declares the map.
pub fn style(p: &Program, doc: u16, spec: Option<&str>) -> Result<Arc<Style>, String> {
    let spec = spec.unwrap_or("protomaps-light");
    if let Some(t) = sr_geo::style::builtin(spec) {
        return reading(&STYLES, PathBuf::from(spec), None, || Style::parse(t).map(Arc::new));
    }
    let path = match sr_model::assets::resolve(spec, &p.doc_base(doc)) {
        sr_model::assets::Resolved::Local(path) => path,
        sr_model::assets::Resolved::Remote(u) => return Err(format!("{u}: styles are read from local files")),
    };
    let read = || {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Style::parse(&text).map(Arc::new)
    };
    reading(&STYLES, path.clone(), Some(stamp(&path)?), read)
}

/// The tile zoom a basemap draws at: raster tiles closest to their pixel size (rounded), vector
/// tiles by the floor of the map zoom as MapLibre does, shifted by `detail`.
pub fn basemap_zoom(proj: &sr_geo::project::Projection, tile_size: f64, detail: f64, raster: bool) -> u8 {
    let z = (std::f64::consts::TAU * proj.scale() / tile_size).log2() + detail;
    let z = if raster { z.round() } else { z.floor() };
    z.clamp(0.0, 24.0) as u8
}

/// How elevation tiles encode heights.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DemEncoding {
    /// Mapzen/AWS Terrarium: metres = R·256 + G + B/256 − 32768.
    Terrarium,
    /// Mapbox Terrain-RGB: metres = −10000 + (R·65536 + G·256 + B)·0.1.
    Mapbox,
}

/// An elevation tile decoded to metres: (width, heights row-major).
pub type Dem = (u32, Vec<f32>);

/// Elevation tile z/x/y of an archive of encoded PNG (or WebP) tiles, in metres (cached).
pub fn dem(a: &Arc<Archive>, z: u8, x: u32, y: u32, enc: DemEncoding) -> Result<Option<Arc<Dem>>, String> {
    type DemCache = HashMap<(usize, u8, u32, u32, DemEncoding), (Weak<Archive>, Option<Arc<Dem>>)>;
    static CACHE: OnceLock<Mutex<DemCache>> = OnceLock::new();
    let key = (Arc::as_ptr(a) as usize, z, x, y, enc);
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, d)) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        return Ok(d.clone());
    }
    let d = match a.tile(z, x, y)? {
        None => None,
        Some(b) => {
            let img = image::load_from_memory(&b).map_err(|e| format!("elevation tile {z}/{x}/{y}: {e}"))?.to_rgb8();
            let w = img.width();
            let h = img
                .pixels()
                .map(|p| {
                    let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
                    match enc {
                        DemEncoding::Terrarium => r * 256.0 + g + b / 256.0 - 32768.0,
                        DemEncoding::Mapbox => -10000.0 + (r * 65536.0 + g * 256.0 + b) * 0.1,
                    }
                })
                .collect();
            Some(Arc::new((w, h)))
        }
    };
    let mut c = cache.lock().unwrap_or_else(|p| p.into_inner());
    if c.len() > 512 {
        c.clear();
    }
    c.insert(key, (Arc::downgrade(a), d.clone()));
    Ok(d)
}

/// Elevation (metres) at a longitude and latitude from the elevation tiles of zoom `z`
/// (bilinear within a tile; `None` where the archive has no tile).
pub fn elevation(a: &Arc<Archive>, z: u8, enc: DemEncoding, lon: f64, lat: f64) -> Result<Option<f64>, String> {
    let [tx, ty] = sr_geo::tiles::tile_xy(z, lon, lat);
    let n = 1u32 << z;
    let (x, y) = ((tx.floor() as i64).clamp(0, n as i64 - 1) as u32, (ty.floor() as i64).clamp(0, n as i64 - 1) as u32);
    let Some(d) = dem(a, z, x, y, enc)? else { return Ok(None) };
    let w = d.0 as usize;
    let hgt = d.1.len() / w.max(1);
    let fx = ((tx - x as f64) * w as f64 - 0.5).clamp(0.0, (w - 1) as f64);
    let fy = ((ty - y as f64) * hgt as f64 - 0.5).clamp(0.0, (hgt - 1) as f64);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(hgt - 1));
    let (u, v) = (fx - x0 as f64, fy - y0 as f64);
    let at = |x: usize, y: usize| d.1[y * w + x] as f64;
    Ok(Some((at(x0, y0) * (1.0 - u) + at(x1, y0) * u) * (1.0 - v) + (at(x0, y1) * (1.0 - u) + at(x1, y1) * u) * v))
}

#[cfg(test)]
mod tests {
    use super::FITS;
    use std::path::{Path, PathBuf};

    /// A scratch directory of this test, removed when dropped.
    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d = std::env::temp_dir().join(format!("sr-eval-geo-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&d).unwrap();
            Dir(d)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn held<K, S, V>(cache: &super::Readings<K, S, V>, matching: impl Fn(&K) -> bool) -> usize {
        cache.get().map_or(0, |c| c.lock().unwrap().keys().filter(|k| matching(k)).count())
    }

    fn boxes(n: usize) -> String {
        let f = r#"{"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[0,0],[40,0],[40,20],[0,20],[0,0]]]}}"#;
        format!(r#"{{"type":"FeatureCollection","features":[{}]}}"#, vec![f; n].join(","))
    }

    #[test]
    fn a_rewritten_geo_file_replaces_its_cached_features() {
        let d = Dir::new("rewrite");
        let path = d.0.join("boxes.geojson");
        for n in 1..=3 {
            std::fs::write(&path, boxes(n)).unwrap();
            assert_eq!(super::load(&path, "geojson", None).unwrap().len(), n, "the rewritten file is read again");
        }
        assert_eq!(held(&super::FEATURES, |k| k.0 == path), 1, "one reading per file, not one per revision");
    }

    fn archive_of(path: &Path, byte: u8) {
        let tile = vec![byte; 16 + byte as usize];
        std::fs::write(
            path,
            sr_geo::pmtiles::write(&[((0, 0, 0), tile)], sr_geo::pmtiles::TileType::Png, 1, &serde_json::json!({})),
        )
        .unwrap();
    }

    #[test]
    fn a_rewritten_archive_replaces_the_open_one_and_its_tiles() {
        let d = Dir::new("archive");
        let path = d.0.join("t.pmtiles");
        for byte in 1..=3u8 {
            archive_of(&path, byte);
            let a = super::archive(&path).unwrap();
            let Some(t) = super::tile(&a, 0, 0, 0).unwrap() else { panic!("tile 0/0/0") };
            let super::TileData::Raster(b) = &*t else { panic!("a raster tile") };
            assert_eq!(**b, vec![byte; 16 + byte as usize], "the tile of the current archive, not a stale one");
        }
        assert_eq!(held(&super::ARCHIVES, |k| *k == path), 1, "one open archive per file, not one per revision");
    }

    #[test]
    fn a_rewritten_style_file_replaces_its_cached_style() {
        let d = Dir::new("style");
        let xml =
            r##"<scene version="1.6"><project width="20" height="20" fps="10" duration="1"/><composition/></scene>"##;
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(d.0.clone()) };
        let doc = sr_model::load_str(xml, &opts).unwrap_or_else(|e| panic!("{e:?}"));
        let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e:?}"));
        let path = d.0.join("s.json");
        for n in 1..=3 {
            let layers: Vec<String> = (0..n).map(|i| format!(r#"{{"id":"l{i}","type":"background"}}"#)).collect();
            std::fs::write(&path, format!(r#"{{"version":8,"layers":[{}]}}"#, layers.join(","))).unwrap();
            let s = super::style(ev.program(), 0, Some(path.to_str().unwrap())).unwrap();
            assert_eq!(s.layers.len(), n, "the rewritten style is read again");
        }
        assert_eq!(held(&super::STYLES, |k| *k == path), 1, "one style per file, not one per revision");
    }

    #[test]
    fn a_rewritten_fit_file_refits_the_map_in_place() {
        let d = Dir::new("refit");
        let xml = r##"<scene version="1.6"><project width="200" height="200" fps="10" duration="1"/>
<assets><geo id="g" src="fit.geojson"/><map id="m" width="200" height="200" projection="equirectangular" fit="g"/></assets>
<composition><shape id="mark" shape="rect" width="4" height="4" fill="#FFFFFF"><expression property="x">geo('m', 20, 10)[0]</expression></shape></composition></scene>"##;
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(d.0.clone()) };
        let doc = sr_model::load_str(xml, &opts).unwrap_or_else(|e| panic!("{e:?}"));
        let mut xs = Vec::new();
        for east in [40, 80, 120] {
            let f = format!(
                r#"{{"type":"FeatureCollection","features":[{{"type":"Feature","properties":{{}},"geometry":{{"type":"Polygon","coordinates":[[[0,0],[{east},0],[{east},20],[0,20],[0,0]]]}}}}]}}"#
            );
            std::fs::write(d.0.join("fit.geojson"), f).unwrap();
            let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e:?}"));
            let g = ev.evaluate_layout(0.0);
            xs.push(g.nodes.iter().find(|n| &*n.id == "mark").expect("mark").world.0[4]);
        }
        assert!(xs[0] > xs[1] && xs[1] > xs[2], "each revision of the fit data refits the map: {xs:?}");
        let dir = d.0.display().to_string();
        assert_eq!(held(&super::CAMERAS, |k| k.contains(&dir)), 1, "one camera per map setup, not one per revision");
    }

    #[test]
    fn a_session_holds_a_bounded_number_of_readings() {
        static CACHE: super::Readings<usize, u8, usize> = std::sync::OnceLock::new();
        for k in 0..3 * super::READINGS {
            assert_eq!(super::reading(&CACHE, k, 0, || Ok(k)), Ok(k));
        }
        assert!(held(&CACHE, |_| true) <= super::READINGS);
        assert_eq!(super::reading(&CACHE, 7, 0, || Ok(7)), Ok(7));
        assert_eq!(super::reading(&CACHE, 7, 1, || Err::<usize, _>("gone".to_string())), Err("gone".into()));
        assert_eq!(held(&CACHE, |k| *k == 7), 0, "a failed read leaves no stale reading behind");
    }

    #[test]
    fn a_program_fits_each_map_once() {
        let d = std::env::temp_dir().join(format!("sr-eval-geo-fit-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("box.geojson"),
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[0,0],[40,0],[40,20],[0,20],[0,0]]]}}]}"#,
        )
        .unwrap();
        let xml = r##"<scene version="1.6"><project width="200" height="200" fps="10" duration="2"/>
<assets><geo id="g" src="box.geojson"/><map id="m" width="200" height="200" projection="equirectangular" fit="g"/></assets>
<composition><shape id="mark" shape="rect" width="4" height="4" fill="#FFFFFF"><expression property="x">geo('m', 20, 10)[0]</expression></shape></composition></scene>"##;
        let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(d.clone()) };
        let doc = sr_model::load_str(xml, &opts).unwrap_or_else(|e| panic!("{e:?}"));
        let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e:?}"));
        FITS.with(|c| c.set(0));
        let xs: Vec<f64> = (0..4)
            .map(|i| {
                let g = ev.evaluate_layout(i as f64 * 0.25);
                let n = g.nodes.iter().find(|n| &*n.id == "mark").expect("mark");
                n.world.0[4]
            })
            .collect();
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(FITS.with(|c| c.get()), 1, "the fit data is looked up once per program, not per expression call");
        assert!((xs[0] - 100.0).abs() < 1.0, "the box's centre is the frame's: {xs:?}");
        assert!(xs.iter().all(|&x| x == xs[0]), "{xs:?}");
    }
}
