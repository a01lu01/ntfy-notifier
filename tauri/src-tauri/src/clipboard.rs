use std::thread;
use std::time::Duration;

pub fn copy_text(text: &str) -> Result<(), String> {
    let mut last_err = String::new();
    for _ in 0..3 {
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())) {
            Ok(_) => return Ok(()),
            Err(e) => last_err = e.to_string(),
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(last_err)
}
