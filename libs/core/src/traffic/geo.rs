//! Places of remote addresses (ADR 11, change 2), from two DB-IP Lite
//! databases in MaxMind format on this Mac:
//!
//! | File | Gives |
//! |---|---|
//! | `<data>/geo/dbip-city-lite.mmdb` | city, country, latitude, longitude |
//! | `<data>/geo/dbip-asn-lite.mmdb` | the network owner, `GitHub, Inc.` |
//!
//! Both are CC BY 4.0: the app credits "IP geolocation by DB-IP". A lookup
//! never uses the network (I11). Only [`install`] downloads, when the user
//! asks, and the daemon gives it a fetch function that uses the verifying
//! TLS settings.

use std::future::Future;
use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use maxminddb::{Mmap, Reader, geoip2};

use crate::api::{GeoDbInfo, Place};

pub const CITY_FILE: &str = "dbip-city-lite.mmdb";
pub const ASN_FILE: &str = "dbip-asn-lite.mmdb";
/// Holds the month of the installed files: `2026-11`.
pub const DATE_FILE: &str = "dbip-month.txt";
/// A download larger than this, after unpacking, is refused.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;

/// The download URLs of one month: city, then ASN.
pub fn download_urls(year: i32, month: u8) -> [String; 2] {
    [
        format!("https://download.db-ip.com/free/dbip-city-lite-{year:04}-{month:02}.mmdb.gz"),
        format!("https://download.db-ip.com/free/dbip-asn-lite-{year:04}-{month:02}.mmdb.gz"),
    ]
}

/// The current UTC year and month.
pub fn current_month() -> (i32, u8) {
    let now = time::OffsetDateTime::now_utc();
    (now.year(), now.month() as u8)
}

pub fn previous_month(year: i32, month: u8) -> (i32, u8) {
    if month <= 1 { (year - 1, 12) } else { (year, month - 1) }
}

/// The databases, opened. Either can be missing.
#[derive(Default)]
pub struct Places {
    city: Option<Reader<Mmap>>,
    asn: Option<Reader<Mmap>>,
}

impl Places {
    /// Open the files in `dir`; a file that is missing or does not open is
    /// left out.
    pub fn open(dir: &Path) -> Self {
        Self { city: open_db(&dir.join(CITY_FILE)), asn: open_db(&dir.join(ASN_FILE)) }
    }

    pub fn is_empty(&self) -> bool {
        self.city.is_none() && self.asn.is_none()
    }

    /// The place and the network owner of an address. No network.
    pub fn lookup(&self, ip: IpAddr) -> (Option<Place>, Option<String>) {
        let place = self.city.as_ref().and_then(|db| {
            let city: geoip2::City = db.lookup(ip).ok()?.decode().ok()??;
            let lat = city.location.latitude?;
            let lon = city.location.longitude?;
            Some(Place {
                city: city.city.names.english.map(str::to_string),
                country: city.country.names.english.map(str::to_string),
                country_code: city.country.iso_code.map(str::to_string),
                lat,
                lon,
            })
        });
        let owner = self.asn.as_ref().and_then(|db| {
            let asn: geoip2::Asn = db.lookup(ip).ok()?.decode().ok()??;
            asn.autonomous_system_organization.map(str::to_string)
        });
        (place, owner)
    }
}

fn open_db(path: &Path) -> Option<Reader<Mmap>> {
    if !path.is_file() {
        return None;
    }
    // SAFETY: the daemon is the only writer of the folder and never writes
    // into a database file: a new one is renamed over it (a new inode), so
    // the mapped file never changes under the reader.
    match unsafe { Reader::open_mmap(path) } {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!("cannot open {}: {e}", path.display());
            None
        }
    }
}

/// The installed month and the size of both files, when there are files.
pub fn info(dir: &Path) -> Option<GeoDbInfo> {
    let size = |name: &str| std::fs::metadata(dir.join(name)).ok().filter(|m| m.is_file()).map(|m| m.len());
    let bytes = size(CITY_FILE).unwrap_or(0) + size(ASN_FILE).unwrap_or(0);
    if bytes == 0 {
        return None;
    }
    let date = std::fs::read_to_string(dir.join(DATE_FILE)).map(|s| s.trim().to_string()).unwrap_or_default();
    Some(GeoDbInfo { date, bytes })
}

/// Download both databases of `year`-`month`, or of the month before when
/// this month's are not there yet (404), into `dir`. Each file is written
/// as `.part`, opened to prove it is a database, then renamed over the old
/// one (I11). A failure leaves the old files as they were.
///
/// `fetch` returns the body, `None` for a 404, or an error.
pub async fn install<F, Fut>(dir: &Path, year: i32, month: u8, fetch: F) -> Result<GeoDbInfo, String>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<Option<Vec<u8>>, String>>,
{
    let mut chosen = None;
    for (y, m) in [(year, month), previous_month(year, month)] {
        let [city_url, asn_url] = download_urls(y, m);
        let Some(city) = fetch(city_url.clone()).await? else { continue };
        let asn = fetch(asn_url.clone()).await?.ok_or_else(|| format!("{asn_url} was not found"))?;
        chosen = Some((y, m, city, asn));
        break;
    }
    let Some((y, m, city, asn)) = chosen else {
        return Err("DB-IP has no database for this month or the month before".into());
    };
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let city_part = write_part(dir, CITY_FILE, &city)?;
    let asn_part = match write_part(dir, ASN_FILE, &asn) {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_file(&city_part);
            return Err(e);
        }
    };
    for (part, name) in [(&city_part, CITY_FILE), (&asn_part, ASN_FILE)] {
        if let Err(e) = std::fs::rename(part, dir.join(name)) {
            let _ = std::fs::remove_file(&city_part);
            let _ = std::fs::remove_file(&asn_part);
            return Err(format!("cannot replace {name}: {e}"));
        }
    }
    let date = format!("{y:04}-{m:02}");
    let _ = std::fs::write(dir.join(DATE_FILE), format!("{date}\n"));
    Ok(info(dir).unwrap_or(GeoDbInfo { date, bytes: 0 }))
}

/// GET `url` through the upstream client, which always verifies the
/// server's certificate (I11). `None` for a 404. The body is limited to
/// 200 MB.
pub async fn fetch(upstream: &crate::upstream::Upstream, url: String) -> Result<Option<Vec<u8>>, String> {
    use http_body_util::{BodyExt, Full, Limited};
    let req = hyper::Request::get(&url)
        .body(Full::new(bytes::Bytes::new()).map_err(|never| match never {}).boxed_unsync())
        .map_err(|e| e.to_string())?;
    let resp = upstream.send(req, false).await.map_err(|e| format!("cannot download {url}: {e}"))?;
    match resp.status().as_u16() {
        200 => {}
        404 => return Ok(None),
        code => return Err(format!("{url} answered {code}")),
    }
    let body = Limited::new(resp.into_body(), 200 * 1024 * 1024)
        .collect()
        .await
        .map_err(|e| format!("cannot download {url}: {e}"))?;
    Ok(Some(body.to_bytes().to_vec()))
}

/// Unpack a `.gz` body into `<name>.part` and open it as a database. On any
/// failure the `.part` file is removed.
fn write_part(dir: &Path, name: &str, gz: &[u8]) -> Result<PathBuf, String> {
    let part = dir.join(format!("{name}.part"));
    let result = (|| {
        let mut data = vec![];
        flate2::read::GzDecoder::new(gz)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| format!("{name}: not a gzip file: {e}"))?;
        if data.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("{name}: larger than {} MB", MAX_FILE_BYTES / 1024 / 1024));
        }
        std::fs::write(&part, &data).map_err(|e| format!("cannot write {}: {e}", part.display()))?;
        Reader::open_readfile(&part).map_err(|e| format!("{name}: the download is not a database: {e}"))?;
        Ok(part.clone())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::Mutex;

    use super::*;

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(vec![], flate2::Compression::fast());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn the_urls_of_a_month() {
        let [city, asn] = download_urls(2026, 11);
        assert_eq!(city, "https://download.db-ip.com/free/dbip-city-lite-2026-11.mmdb.gz");
        assert_eq!(asn, "https://download.db-ip.com/free/dbip-asn-lite-2026-11.mmdb.gz");
        assert_eq!(previous_month(2026, 11), (2026, 10));
        assert_eq!(previous_month(2027, 1), (2026, 12));
    }

    #[tokio::test]
    async fn a_404_falls_back_to_the_month_before() {
        let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
        let asked = Mutex::new(vec![]);
        let fetch = |url: String| {
            asked.lock().unwrap().push(url.clone());
            async move { if url.contains("2026-11") { Ok(None) } else { Ok(Some(gz(b"not a database"))) } }
        };
        let e = install(dir.path(), 2026, 11, fetch).await.unwrap_err();
        let asked = asked.lock().unwrap().clone();
        assert_eq!(asked[0], download_urls(2026, 11)[0]);
        assert_eq!(asked[1], download_urls(2026, 10)[0]);
        assert!(e.contains("not a database"), "{e}");
    }

    #[tokio::test]
    async fn a_download_that_does_not_open_leaves_the_old_file_and_no_part() {
        let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
        std::fs::write(dir.path().join(CITY_FILE), b"old city").unwrap();
        std::fs::write(dir.path().join(ASN_FILE), b"old asn").unwrap();
        let fetch = |_url: String| async { Ok(Some(gz(b"garbage"))) };
        assert!(install(dir.path(), 2026, 11, fetch).await.is_err());
        assert_eq!(std::fs::read(dir.path().join(CITY_FILE)).unwrap(), b"old city");
        assert_eq!(std::fs::read(dir.path().join(ASN_FILE)).unwrap(), b"old asn");
        let names: Vec<String> =
            std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(!names.iter().any(|n| n.ends_with(".part")), "{names:?}");
        // A body that is not gzip at all, and a network error, change nothing too.
        let fetch = |_url: String| async { Ok(Some(b"plain".to_vec())) };
        assert!(install(dir.path(), 2026, 11, fetch).await.unwrap_err().contains("not a gzip"));
        let fetch = |_url: String| async { Err::<Option<Vec<u8>>, _>("offline".to_string()) };
        assert_eq!(install(dir.path(), 2026, 11, fetch).await.unwrap_err(), "offline");
        assert_eq!(std::fs::read(dir.path().join(CITY_FILE)).unwrap(), b"old city");
    }

    #[test]
    fn info_and_empty_places() {
        let dir = tempfile::Builder::new().prefix("lr").tempdir().unwrap();
        assert_eq!(info(dir.path()), None);
        assert!(Places::open(dir.path()).is_empty());
        // A file that is not a database is left out, not a crash.
        std::fs::write(dir.path().join(CITY_FILE), b"x").unwrap();
        std::fs::write(dir.path().join(DATE_FILE), "2026-10\n").unwrap();
        assert!(Places::open(dir.path()).is_empty());
        assert_eq!(info(dir.path()), Some(GeoDbInfo { date: "2026-10".into(), bytes: 1 }));
        assert_eq!(Places::default().lookup("203.0.113.4".parse().unwrap()), (None, None));
    }
}
