#include "blockloom_application.h"

#include "cxx-qt-lib/qcoreapplication.h"
#include <QtWidgets/QApplication>

std::unique_ptr<QGuiApplication> blockloom_new_application(const QVector<QByteArray> &args)
{
    auto arguments = std::make_unique<rust::cxxqtlib1::ApplicationArgsData>(args);
    // KDE's native dialog helpers need QApplication even in a Qt Quick app.
    auto app = std::make_unique<QApplication>(arguments->size(), arguments->data());
    arguments.release()->setParent(app.get());
    return app;
}
