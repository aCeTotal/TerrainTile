use std::io::{BufWriter, Read};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Inner raster paths (.tif/.tiff) in a zip, e.g. hoydedata.no exports.
pub fn list_rasters(zip_path: &Path) -> Result<Vec<String>> {
    let file = std::fs::File::open(zip_path)
        .with_context(|| format!("kan ikke åpne {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("{}: ugyldig zip", zip_path.display()))?;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i)?;
        let name = entry.name().to_string();
        let lower = name.to_lowercase();
        if lower.ends_with(".tif") || lower.ends_with(".tiff") {
            out.push(name);
        }
    }
    if out.is_empty() {
        bail!("{}: ingen .tif-filer i arkivet", zip_path.display());
    }
    out.sort();
    Ok(out)
}

/// Extract one raster (plus any sidecars like .tfw/.prj) to `dest_dir`,
/// streaming — RAM use is one IO buffer regardless of file size. Skips work
/// already done: an existing file with the right size is reused (resume).
/// Returns the extracted raster path.
pub fn extract(zip_path: &Path, inner: &str, dest_dir: &Path) -> Result<PathBuf> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    let stem = inner.rsplit('/').next().unwrap().to_string();
    let raster_dest = dest_dir.join(&stem);
    extract_entry(&mut archive, inner, &raster_dest)?;

    // Sidecars share the path minus extension (x.tif -> x.tfw, x.prj, ...).
    let prefix = inner.rsplit_once('.').map(|(p, _)| format!("{p}.")).unwrap_or_default();
    let sidecars: Vec<String> = (0..archive.len())
        .filter_map(|i| {
            let name = archive.by_index_raw(i).ok()?.name().to_string();
            (name.starts_with(&prefix) && name != inner).then_some(name)
        })
        .collect();
    for name in sidecars {
        let dest = dest_dir.join(name.rsplit('/').next().unwrap());
        extract_entry(&mut archive, &name, &dest)?;
    }
    Ok(raster_dest)
}

fn extract_entry(
    archive: &mut zip::ZipArchive<std::fs::File>,
    inner: &str,
    dest: &Path,
) -> Result<()> {
    let mut entry = archive
        .by_name(inner)
        .with_context(|| format!("{inner} finnes ikke i arkivet"))?;
    if let Ok(meta) = std::fs::metadata(dest) {
        if meta.len() == entry.size() {
            return Ok(());
        }
    }
    std::fs::create_dir_all(dest.parent().unwrap())?;
    let tmp = dest.with_extension("part");
    let mut w = BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
    std::io::copy(&mut entry.by_ref(), &mut w)
        .with_context(|| format!("utpakking av {inner} feilet"))?;
    drop(w);
    std::fs::rename(&tmp, dest)?;
    Ok(())
}
