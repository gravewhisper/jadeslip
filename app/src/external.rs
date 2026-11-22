#[cfg(not(feature = "e2e"))]
pub fn open(url: &str) {
    if let Ok(mut child) = std::process::Command::new("xdg-open").arg(url).spawn() {
        std::thread::spawn(move || child.wait());
    }
}

#[cfg(feature = "e2e")]
pub fn open(url: &str) {
    crate::e2e::record_external(url);
}

#[cfg(not(feature = "e2e"))]
pub fn reveal(folder: &std::path::Path) {
    if let Ok(mut child) = std::process::Command::new("xdg-open").arg(folder).spawn() {
        std::thread::spawn(move || child.wait());
    }
}

#[cfg(feature = "e2e")]
pub fn reveal(folder: &std::path::Path) {
    crate::e2e::record_external(&format!("reveal:{}", folder.display()));
}
