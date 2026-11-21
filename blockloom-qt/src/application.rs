#[path = "../../qt-common/application_args.rs"]
mod args;

#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qguiapplication.h");
        type QGuiApplication = cxx_qt_lib::QGuiApplication;
        include!("cxx-qt-lib/core/qvector/qvector_QByteArray.h");
        type QVector_QByteArray = cxx_qt_lib::QVector<cxx_qt_lib::QByteArray>;
        include!("blockloom_application.h");

        fn blockloom_new_application(args: &QVector_QByteArray) -> UniquePtr<QGuiApplication>;
    }
}

pub fn new() -> cxx::UniquePtr<cxx_qt_lib::QGuiApplication> {
    ffi::blockloom_new_application(&args::application_args())
}
