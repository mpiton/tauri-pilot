const COMMANDS: &[&str] = &["__callback"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
