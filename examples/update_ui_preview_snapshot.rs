use std::fs;
use std::path::Path;

fn main() -> std::io::Result<()> {
    for snapshot in pacopt::ui_preview::snapshots() {
        if let Some(parent) = Path::new(snapshot.path).parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(snapshot.path, snapshot.contents)?;
    }

    Ok(())
}
