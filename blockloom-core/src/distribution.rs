//! Platform branding and the ZIP a finished build is shared as.

use crc32fast::Hasher;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageFormat, RgbaImage};
use std::fs::File;
use std::io::{Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};

const DEFAULT_ICON: &[u8] = include_bytes!("../../src-tauri/icons/512x512.png");

pub struct Icons {
    pub png: Vec<u8>,
    pub ico: Vec<u8>,
    pub icns: Vec<u8>,
}

impl Icons {
    pub fn load(project_dir: &Path, relative: &str) -> Result<Self, String> {
        let bytes = if relative.trim().is_empty() {
            DEFAULT_ICON.to_vec()
        } else {
            let path = crate::assets::resolve(project_dir, relative)
                .ok_or_else(|| format!("\"{relative}\" isn't a path in this project"))?;
            std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?
        };
        let image = image::load_from_memory(&bytes)
            .map_err(|error| format!("the game icon could not be read: {error}"))?;
        Ok(Self {
            png: png(&image, 512)?,
            ico: ico(&image)?,
            icns: icns(&image)?,
        })
    }
}

/// The centered square fit every branded icon uses, exposed for the
/// Android launcher densities (same file, different module).
pub(crate) fn square_for_launcher(image: &DynamicImage, size: u32) -> RgbaImage {
    square(image, size)
}

fn square(image: &DynamicImage, size: u32) -> RgbaImage {
    let (width, height) = image.dimensions();
    let scale = (size as f64 / width as f64).min(size as f64 / height as f64);
    let width = ((width as f64 * scale).round() as u32).clamp(1, size);
    let height = ((height as f64 * scale).round() as u32).clamp(1, size);
    let resized = image
        .resize_exact(width, height, FilterType::Lanczos3)
        .to_rgba8();
    let mut canvas = RgbaImage::new(size, size);
    image::imageops::overlay(
        &mut canvas,
        &resized,
        i64::from((size - width) / 2),
        i64::from((size - height) / 2),
    );
    canvas
}

fn png(image: &DynamicImage, size: u32) -> Result<Vec<u8>, String> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(square(image, size))
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(|error| format!("couldn't encode the game icon: {error}"))?;
    Ok(bytes.into_inner())
}

fn ico(image: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut directory = IconDir::new(ResourceType::Icon);
    for size in [16, 24, 32, 48, 64, 128, 256] {
        let rgba = square(image, size);
        let icon = IconImage::from_rgba_data(size, size, rgba.into_raw());
        let entry = IconDirEntry::encode(&icon)
            .map_err(|error| format!("couldn't encode the Windows icon: {error}"))?;
        directory.add_entry(entry);
    }
    let mut bytes = Vec::new();
    directory
        .write(&mut bytes)
        .map_err(|error| format!("couldn't write the Windows icon: {error}"))?;
    Ok(bytes)
}

fn icns(image: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut elements = Vec::new();
    for (kind, size) in [
        (*b"icp4", 16),
        (*b"icp5", 32),
        (*b"icp6", 64),
        (*b"ic07", 128),
        (*b"ic08", 256),
        (*b"ic09", 512),
        (*b"ic10", 1024),
    ] {
        let data = png(image, size)?;
        let length =
            u32::try_from(data.len() + 8).map_err(|_| "the macOS icon is too large".to_string())?;
        elements.extend_from_slice(&kind);
        elements.extend_from_slice(&length.to_be_bytes());
        elements.extend_from_slice(&data);
    }
    let length =
        u32::try_from(elements.len() + 8).map_err(|_| "the macOS icon is too large".to_string())?;
    let mut bytes = Vec::with_capacity(length as usize);
    bytes.extend_from_slice(b"icns");
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(&elements);
    Ok(bytes)
}

#[cfg(windows)]
pub fn apply_windows_icon(binary: &Path, icon: &[u8]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{
        BeginUpdateResourceW, EndUpdateResourceW, UpdateResourceW,
    };

    let directory = IconDir::read(Cursor::new(icon))
        .map_err(|error| format!("couldn't read the Windows icon: {error}"))?;
    let path: Vec<u16> = binary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe { BeginUpdateResourceW(path.as_ptr(), 0) };
    if handle.is_null() {
        return Err(format!(
            "couldn't open {} to set its icon: {}",
            binary.display(),
            std::io::Error::last_os_error()
        ));
    }

    let update = || -> Result<(), String> {
        let mut group = Vec::new();
        group.extend_from_slice(&0_u16.to_le_bytes());
        group.extend_from_slice(&1_u16.to_le_bytes());
        group.extend_from_slice(&(directory.entries().len() as u16).to_le_bytes());
        for (index, entry) in directory.entries().iter().enumerate() {
            let id = u16::try_from(index + 1).map_err(|_| "too many icon sizes".to_string())?;
            let width = if entry.width() >= 256 {
                0
            } else {
                entry.width() as u8
            };
            let height = if entry.height() >= 256 {
                0
            } else {
                entry.height() as u8
            };
            group.extend_from_slice(&[width, height, 0, 0]);
            group.extend_from_slice(&1_u16.to_le_bytes());
            group.extend_from_slice(&entry.bits_per_pixel().to_le_bytes());
            group.extend_from_slice(&(entry.data().len() as u32).to_le_bytes());
            group.extend_from_slice(&id.to_le_bytes());
            let ok = unsafe {
                UpdateResourceW(
                    handle,
                    resource_id(3),
                    resource_id(id),
                    0,
                    entry.data().as_ptr().cast(),
                    entry.data().len() as u32,
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        let ok = unsafe {
            UpdateResourceW(
                handle,
                resource_id(14),
                resource_id(1),
                0,
                group.as_ptr().cast(),
                group.len() as u32,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }();
    if let Err(error) = update {
        unsafe { EndUpdateResourceW(handle, 1) };
        return Err(format!(
            "couldn't set the icon on {}: {error}",
            binary.display()
        ));
    }
    if unsafe { EndUpdateResourceW(handle, 0) } == 0 {
        return Err(format!(
            "couldn't save the icon on {}: {}",
            binary.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn resource_id(id: u16) -> *const u16 {
    std::ptr::without_provenance(usize::from(id))
}

#[cfg(not(windows))]
pub fn apply_windows_icon(_binary: &Path, _icon: &[u8]) -> Result<(), String> {
    Ok(())
}

struct ZipEntry {
    name: Vec<u8>,
    crc: u32,
    compressed: u32,
    uncompressed: u32,
    offset: u32,
    mode: u32,
}

/// Creates a portable ZIP with the build folder as its one top-level entry.
pub fn archive(root: &Path, destination: &Path, executables: &[PathBuf]) -> Result<(), String> {
    let mut files = Vec::new();
    collect_files(root, &mut files)?;
    let parent = root.parent().unwrap_or_else(|| Path::new(""));
    let partial = destination.with_extension("zip.part");
    struct PartialArchive(PathBuf);
    impl Drop for PartialArchive {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _partial = PartialArchive(partial.clone());
    let mut output =
        File::create(&partial).map_err(|error| format!("{}: {error}", partial.display()))?;
    let mut central = Vec::new();
    for path in files {
        crate::build_control::check()?;
        let relative = path.strip_prefix(parent).unwrap_or(&path);
        let name = relative.to_string_lossy().replace('\\', "/").into_bytes();
        if name.len() > usize::from(u16::MAX) {
            return Err(format!("{} has a path too long for ZIP", path.display()));
        }
        let mut data = Vec::new();
        File::open(&path)
            .and_then(|mut file| file.read_to_end(&mut data))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let mut hasher = Hasher::new();
        hasher.update(&data);
        let crc = hasher.finalize();
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        for chunk in data.chunks(64 * 1024) {
            crate::build_control::check()?;
            encoder
                .write_all(chunk)
                .map_err(|error| format!("couldn't compress {}: {error}", path.display()))?;
        }
        let compressed = encoder
            .finish()
            .map_err(|error| format!("couldn't compress {}: {error}", path.display()))?;
        let offset = u32::try_from(output.stream_position().map_err(|e| e.to_string())?)
            .map_err(|_| "the ZIP is larger than 4 GB".to_string())?;
        let compressed_len =
            u32::try_from(compressed.len()).map_err(|_| "a ZIP entry is larger than 4 GB")?;
        let uncompressed_len =
            u32::try_from(data.len()).map_err(|_| "a ZIP entry is larger than 4 GB")?;
        write_local_header(&mut output, &name, crc, compressed_len, uncompressed_len)?;
        output
            .write_all(&compressed)
            .map_err(|error| format!("{}: {error}", partial.display()))?;
        let executable = executables.iter().any(|candidate| candidate == &path)
            || path.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("so")
                    || ext.eq_ignore_ascii_case("dylib")
                    || ext.eq_ignore_ascii_case("desktop")
            });
        central.push(ZipEntry {
            name,
            crc,
            compressed: compressed_len,
            uncompressed: uncompressed_len,
            offset,
            mode: if executable { 0o100755 } else { 0o100644 },
        });
    }
    if central.len() > usize::from(u16::MAX) {
        return Err("the ZIP contains too many files".to_string());
    }
    let central_start = output
        .stream_position()
        .map_err(|error| error.to_string())?;
    for entry in &central {
        write_central_header(&mut output, entry)?;
    }
    let central_end = output
        .stream_position()
        .map_err(|error| error.to_string())?;
    write_end(
        &mut output,
        central.len() as u16,
        u32::try_from(central_end - central_start)
            .map_err(|_| "the ZIP directory is larger than 4 GB")?,
        u32::try_from(central_start).map_err(|_| "the ZIP is larger than 4 GB")?,
    )?;
    output.flush().map_err(|error| error.to_string())?;
    // Closed before the rename, which Windows needs. A no-op in a browser.
    #[cfg_attr(target_arch = "wasm32", allow(clippy::drop_non_drop))]
    drop(output);
    crate::build_control::check()?;
    if destination.exists() {
        std::fs::remove_file(destination)
            .map_err(|error| format!("{}: {error}", destination.display()))?;
    }
    crate::build_control::check()?;
    std::fs::rename(&partial, destination).map_err(|error| {
        format!(
            "{} -> {}: {error}",
            partial.display(),
            destination.display()
        )
    })
}

pub(crate) fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|error| format!("{}: {error}", dir.display()))?
        .collect::<Result<_, _>>()
        .map_err(|error| format!("{}: {error}", dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

fn write_local_header(
    output: &mut File,
    name: &[u8],
    crc: u32,
    compressed: u32,
    uncompressed: u32,
) -> Result<(), String> {
    write_u32(output, 0x0403_4b50)?;
    write_u16(output, 20)?;
    write_u16(output, 0x0800)?;
    write_u16(output, 8)?;
    write_u16(output, 0)?;
    write_u16(output, 33)?;
    write_u32(output, crc)?;
    write_u32(output, compressed)?;
    write_u32(output, uncompressed)?;
    write_u16(output, name.len() as u16)?;
    write_u16(output, 0)?;
    output.write_all(name).map_err(|error| error.to_string())
}

fn write_central_header(output: &mut File, entry: &ZipEntry) -> Result<(), String> {
    write_u32(output, 0x0201_4b50)?;
    write_u16(output, 0x0314)?;
    write_u16(output, 20)?;
    write_u16(output, 0x0800)?;
    write_u16(output, 8)?;
    write_u16(output, 0)?;
    write_u16(output, 33)?;
    write_u32(output, entry.crc)?;
    write_u32(output, entry.compressed)?;
    write_u32(output, entry.uncompressed)?;
    write_u16(output, entry.name.len() as u16)?;
    write_u16(output, 0)?;
    write_u16(output, 0)?;
    write_u16(output, 0)?;
    write_u16(output, 0)?;
    write_u32(output, entry.mode << 16)?;
    write_u32(output, entry.offset)?;
    output
        .write_all(&entry.name)
        .map_err(|error| error.to_string())
}

fn write_end(output: &mut File, count: u16, size: u32, offset: u32) -> Result<(), String> {
    write_u32(output, 0x0605_4b50)?;
    write_u16(output, 0)?;
    write_u16(output, 0)?;
    write_u16(output, count)?;
    write_u16(output, count)?;
    write_u32(output, size)?;
    write_u32(output, offset)?;
    write_u16(output, 0)
}

fn write_u16(output: &mut File, value: u16) -> Result<(), String> {
    output
        .write_all(&value.to_le_bytes())
        .map_err(|error| error.to_string())
}

fn write_u32(output: &mut File, value: u32) -> Result<(), String> {
    output
        .write_all(&value.to_le_bytes())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "blockloom-distribution-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn one_source_image_becomes_every_platform_icon() {
        let root = temp("icons");
        let icons = Icons::load(&root, "").unwrap();

        assert!(icons.png.starts_with(b"\x89PNG"));
        assert_eq!(&icons.ico[..4], &[0, 0, 1, 0]);
        assert!(icons.icns.starts_with(b"icns"));
        assert_eq!(
            u32::from_be_bytes(icons.icns[4..8].try_into().unwrap()) as usize,
            icons.icns.len()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_archive_opens_as_a_standard_zip() {
        let root = temp("zip");
        let build = root.join("Pond Game (Linux x64)");
        std::fs::create_dir_all(build.join("game/assets")).unwrap();
        let binary = build.join("Pond Game");
        std::fs::write(&binary, b"player").unwrap();
        std::fs::write(build.join("game/game.pack"), b"pack").unwrap();
        std::fs::write(build.join("game/assets/pond.txt"), b"ripples").unwrap();
        let destination = root.join("pond.zip");

        archive(&build, &destination, &[binary]).unwrap();

        assert_eq!(&std::fs::read(&destination).unwrap()[..2], b"PK");
        // Read it back with an independent reader, not the system `tar`.
        let mut zip = zip::ZipArchive::new(File::open(&destination).unwrap()).unwrap();
        let mut names: Vec<_> = zip.file_names().map(str::to_owned).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "Pond Game (Linux x64)/Pond Game",
                "Pond Game (Linux x64)/game/assets/pond.txt",
                "Pond Game (Linux x64)/game/game.pack",
            ]
        );
        for (name, contents, mode) in [
            ("Pond Game (Linux x64)/Pond Game", "player", 0o100755),
            ("Pond Game (Linux x64)/game/game.pack", "pack", 0o100644),
            (
                "Pond Game (Linux x64)/game/assets/pond.txt",
                "ripples",
                0o100644,
            ),
        ] {
            let mut entry = zip.by_name(name).unwrap();
            assert_eq!(entry.unix_mode(), Some(mode), "{name}");
            let mut data = String::new();
            entry.read_to_string(&mut data).unwrap();
            assert_eq!(data, contents, "{name}");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
