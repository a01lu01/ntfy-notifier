use winrt_notification::{Sound, Toast};

pub fn show(title: &str, message: &str, app_id: &str) -> bool {
    let result = Toast::new(app_id)
        .title(title)
        .text1(message)
        .sound(Some(Sound::Default))
        .show();
    match result {
        Ok(_) => true,
        Err(_) => {
            message_box(title, message);
            false
        }
    }
}

fn message_box(title: &str, message: &str) {
    #[cfg(windows)]
    {
        use winapi::um::winuser::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
        let title_w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let msg_w: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            MessageBoxW(std::ptr::null_mut(), msg_w.as_ptr(), title_w.as_ptr(), MB_OK | MB_ICONINFORMATION);
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("[ntfy-Notifier 通知] {title}: {message}");
    }
}
