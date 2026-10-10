//! Launcher integration on Linux, so the dock and the app grid show the
//! aispice icon rather than a generic one.
//!
//! GNOME and KDE match a window to its launcher by the window's class (X11)
//! or app id (Wayland). Tauri sets the Wayland app id to the bundle
//! identifier, and GTK takes the X11 class from the program name, so the
//! program name is set to the identifier too and the installed launcher
//! names it in `StartupWMClass`.
//!
//! The Debian and RPM packages install that launcher. An AppImage or a
//! binary built from source has none, and the desktop shows a gear, so on
//! first start such a build writes one for the current user, pointing at
//! itself. `AISPICE_NO_DESKTOP_ENTRY=1` turns that off.

use std::path::{Path, PathBuf};

/// The bundle identifier in tauri.conf.json.
pub const APP_ID: &str = "io.github.danieltyukov.aispice";

const ICONS: &[(u32, &[u8])] = &[
    (32, include_bytes!("../icons/32x32.png")),
    (64, include_bytes!("../icons/64x64.png")),
    (128, include_bytes!("../icons/128x128.png")),
    (256, include_bytes!("../icons/256x256.png")),
    (512, include_bytes!("../icons/512x512.png")),
];

/// Call before Tauri starts GTK.
pub fn prepare() {
    glib::set_prgname(Some(APP_ID));
    if std::env::var_os("AISPICE_NO_DESKTOP_ENTRY").is_some() {
        return;
    }
    if let Err(e) = install_if_missing() {
        eprintln!("aispice: could not register the launcher: {e}");
    }
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = dirs::data_dir().into_iter().collect();
    let system =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(
        system
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    dirs
}

/// The launcher this module writes for the current user.
fn own_entry(data: &Path) -> PathBuf {
    data.join(format!("applications/{APP_ID}.desktop"))
}

/// Whether a launcher other than `own` already claims this app's windows,
/// such as the one the Debian package installs.
fn other_launcher_exists(own: Option<&Path>) -> bool {
    let wanted = format!("StartupWMClass={APP_ID}");
    data_dirs().iter().any(|d| {
        std::fs::read_dir(d.join("applications"))
            .map(|entries| {
                entries.flatten().any(|e| {
                    let path = e.path();
                    Some(path.as_path()) != own
                        && path.extension().is_some_and(|x| x == "desktop")
                        && std::fs::read_to_string(&path)
                            .is_ok_and(|text| text.lines().any(|l| l.trim() == wanted))
                })
            })
            .unwrap_or(false)
    })
}

fn install_if_missing() -> std::io::Result<()> {
    let Some(data) = dirs::data_dir() else {
        return Ok(());
    };
    let own = own_entry(&data);
    if other_launcher_exists(Some(&own)) {
        // Installed from a package since: drop the user launcher written by
        // an earlier AppImage or source build, so the app is listed once.
        if own.exists() {
            std::fs::remove_file(&own)?;
            for (size, _) in ICONS {
                let _ = std::fs::remove_file(
                    data.join(format!("icons/hicolor/{size}x{size}/apps/{APP_ID}.png")),
                );
            }
        }
        return Ok(());
    }
    // An AppImage runs from a temporary mount; $APPIMAGE is the file itself.
    let exe = match std::env::var_os("APPIMAGE") {
        Some(p) => PathBuf::from(p),
        None => std::env::current_exe()?,
    };
    for (size, png) in ICONS {
        let dir = data.join(format!("icons/hicolor/{size}x{size}/apps"));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("{APP_ID}.png")), png)?;
    }
    std::fs::create_dir_all(data.join("applications"))?;
    // Rewritten on every start, so a moved AppImage keeps a working launcher.
    std::fs::write(&own, entry(&exe))
}

fn entry(exe: &Path) -> String {
    // Desktop entry Exec values quote with double quotes and escape `"`,
    // `` ` ``, `$` and `\` inside them.
    let quoted: String = exe
        .display()
        .to_string()
        .chars()
        .flat_map(|c| match c {
            '"' | '`' | '$' | '\\' => vec!['\\', c],
            c => vec![c],
        })
        .collect();
    format!(
        "[Desktop Entry]\nType=Application\nName=aispice\nGenericName=Circuit design assistant\nComment=AI agent for SPICE circuit design\nExec=\"{quoted}\"\nIcon={APP_ID}\nStartupWMClass={APP_ID}\nCategories=Development;Electronics;Engineering;\nKeywords=spice;ltspice;ngspice;circuit;simulation;schematic;\nTerminal=false\nStartupNotify=true\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_the_app_and_quotes_the_path() {
        let e = entry(Path::new("/home/u/My Apps/aispice$1.AppImage"));
        assert!(
            e.contains("Exec=\"/home/u/My Apps/aispice\\$1.AppImage\""),
            "{e}"
        );
        assert!(e.contains(&format!("StartupWMClass={APP_ID}")));
        assert!(e.contains(&format!("Icon={APP_ID}")));
    }
}
