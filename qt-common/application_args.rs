use cxx_qt_lib::{QByteArray, QVector};

pub fn application_args() -> QVector<QByteArray> {
    let mut args = QVector::<QByteArray>::default();
    for arg in std::env::args_os() {
        #[cfg(unix)]
        use std::os::unix::ffi::OsStrExt;
        #[cfg(windows)]
        let arg = arg.to_string_lossy();
        args.append(QByteArray::from(arg.as_bytes()));
    }
    args
}
