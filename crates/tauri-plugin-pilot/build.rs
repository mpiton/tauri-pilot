const COMMANDS: &[&str] = &["callback", "__callback"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
