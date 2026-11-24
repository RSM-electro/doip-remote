//! Embeds `icon.png` and a UAC manifest into the Windows executable.
//!
//! The source image is a large PNG (1254×1254), while the Windows resource
//! format wants an `.ico`. This script resizes the image to the standard icon
//! sizes, writes a PNG-compressed multi-size ICO into `OUT_DIR`, writes a
//! manifest that requires administrator privileges, and lets `embed-resource`
//! compile a tiny `.rc` file and link the result.
//!
//! On non-Windows targets the script does nothing. When cross-compiling for
//! Windows from Linux without an `rc`/`windres` tool, `embed-resource` reports
//! that it could not attempt compilation (`NotAttempted`): the build still
//! succeeds and the resources are embedded automatically on a real Windows host.

use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

/// Icon sizes packed into the ICO (Windows scales from the closest one).
const ICON_SIZES: [u32; 6] = [16, 32, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed=icon.png");
    println!("cargo:rerun-if-changed=build.rs");

    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    let source = manifest_dir.join("icon.png");
    let icon = out_dir.join("doip-remote.ico");

    if let Err(error) = png_to_ico(&source, &icon) {
        // A missing icon must not break the build; surface a clear warning.
        println!(
            "cargo:warning=could not prepare icon {}: {error}",
            source.display()
        );
        return;
    }

    let manifest = out_dir.join("doip-remote.manifest");
    fs::write(&manifest, windows_manifest()).expect("write Windows manifest");

    let resource = out_dir.join("doip-remote.rc");
    fs::write(
        &resource,
        "1 ICON \"doip-remote.ico\"\n1 24 \"doip-remote.manifest\"\n",
    )
    .expect("write resource script");

    let host = env::var("HOST").unwrap_or_default();
    let result = embed_resource::compile(&resource, embed_resource::NONE);

    if host.contains("windows") {
        result.manifest_required().unwrap_or_else(|error| {
            panic!("failed to compile Windows resources: {error}");
        });
    } else {
        match result {
            embed_resource::CompilationResult::Ok
            | embed_resource::CompilationResult::NotWindows => {}
            not_attempted => {
                println!(
                    "cargo:warning={not_attempted}; the icon and manifest will be embedded when the \
                     executable is built on Windows"
                );
            }
        }
    }
}

/// Converts the source PNG into a multi-size, PNG-compressed ICO.
fn png_to_ico(source: &Path, destination: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use image::{codecs::png::PngEncoder, imageops::FilterType, ExtendedColorType, ImageEncoder};

    let source_image = image::open(source)?;
    let mut images = Vec::with_capacity(ICON_SIZES.len());

    for size in ICON_SIZES {
        let resized = source_image.resize_exact(size, size, FilterType::Lanczos3);
        let rgba = resized.to_rgba8();
        let mut png = Vec::new();
        PngEncoder::new(&mut png).write_image(
            rgba.as_raw(),
            size,
            size,
            ExtendedColorType::Rgba8,
        )?;
        images.push((size, png));
    }

    write_ico(destination, &images)?;
    Ok(())
}

/// Windows application manifest requiring administrator privileges (UAC).
fn windows_manifest() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#
}

/// Writes an ICO container with one PNG-compressed entry per image.
///
/// Windows Vista and newer read PNG entries directly, which keeps the resource
/// small and avoids a separate BMP/AND-mask encoder.
fn write_ico(destination: &Path, images: &[(u32, Vec<u8>)]) -> io::Result<()> {
    let mut header = Vec::with_capacity(6 + images.len() * 16);
    header.extend_from_slice(&0_u16.to_le_bytes()); // reserved
    header.extend_from_slice(&1_u16.to_le_bytes()); // type: icon
    header.extend_from_slice(&(images.len() as u16).to_le_bytes());

    let mut offset = 6 + images.len() as u32 * 16;
    let mut directory = Vec::with_capacity(images.len() * 16);

    for (size, data) in images {
        // 0 means 256 in the ICO directory.
        let dimension = if *size >= 256 { 0 } else { *size as u8 };
        directory.push(dimension); // width
        directory.push(dimension); // height
        directory.push(0); // palette entries
        directory.push(0); // reserved
        directory.extend_from_slice(&1_u16.to_le_bytes()); // color planes
        directory.extend_from_slice(&32_u16.to_le_bytes()); // bits per pixel
        directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }

    let mut file = fs::File::create(destination)?;
    file.write_all(&header)?;
    file.write_all(&directory)?;
    for (_, data) in images {
        file.write_all(data)?;
    }
    Ok(())
}
