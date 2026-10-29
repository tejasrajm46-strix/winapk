fn main() {
    // tauri-build embeds icons/icon.ico into the Windows executable and the
    // installer. It watches tauri.conf.json but not the icon files, so a
    // replaced icon kept the old resource in a cached object file (observed:
    // the installer had the new mark while droidwin.exe still had the old one).
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
