// Gives savepointd.exe its icon and the version details Windows shows in
// file properties and in the Task Manager.
fn main() {
    println!("cargo:rerun-if-changed=../../ui/src-tauri/icons/icon.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../ui/src-tauri/icons/icon.ico")
        .set("ProductName", "Savepoint")
        .set("FileDescription", "Savepoint background service")
        .set("LegalCopyright", "Copyright (c) 2026 eneskocamaan. MIT License.");
    if let Err(e) = res.compile() {
        // Builds still work without the icon (for example without a resource compiler).
        println!("cargo:warning=could not embed the icon: {e}");
    }
}
