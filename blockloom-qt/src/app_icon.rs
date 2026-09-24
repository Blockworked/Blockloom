#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("app_icon.h");

        fn blockloom_apply_window_icon();
        fn blockloom_apply_window_icon_to_windows();
    }
}

pub fn apply() {
    ffi::blockloom_apply_window_icon();
}

pub fn apply_to_windows() {
    ffi::blockloom_apply_window_icon_to_windows();
}
