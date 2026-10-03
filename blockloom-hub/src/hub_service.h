#ifndef BLOCKLOOM_HUB_SERVICE_H
#define BLOCKLOOM_HUB_SERVICE_H

#include <QtCore/QObject>
#include <QtCore/QProcess>
#include <QtCore/QTemporaryDir>
#include <QtCore/QUrl>
#include <QtQml/qqmlregistration.h>

class HubService : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(bool busy READ busy NOTIFY busyChanged)
    Q_PROPERTY(QString log READ log NOTIFY logChanged)
    Q_PROPERTY(bool canCancel READ canCancel NOTIFY busyChanged)
    Q_PROPERTY(bool cancelling READ cancelling NOTIFY cancellingChanged)

public:
    explicit HubService(QObject *parent = nullptr);
    bool busy() const { return m_busy; }
    QString log() const { return m_log; }
    bool canCancel() const;
    bool cancelling() const { return m_cancelRequested; }
    Q_INVOKABLE void cancel();
    Q_INVOKABLE void run(const QString &command, const QStringList &arguments);
    Q_INVOKABLE QString localPath(const QUrl &url) const;
    Q_INVOKABLE bool showBackup(const QString &path) const;
    Q_INVOKABLE bool showFolder(const QString &path) const;
    Q_INVOKABLE bool smokeTest() const;
    Q_INVOKABLE QString smokePage() const;
    Q_INVOKABLE void smokeReady(int projects, int installations);

Q_SIGNALS:
    void busyChanged();
    void logChanged();
    void cancellingChanged();
    void completed(const QString &command, bool ok, const QString &response);

private:
    void finishSmoke(int projects, int installations);
    void appendLog(const QString &text);
    void finish(bool ok, const QString &error = {});
    QTemporaryDir m_scripts;
    QProcess m_process;
    QString m_command;
    QString m_log;
    QByteArray m_output;
    QString m_errorOutput;
    bool m_busy = false;
    bool m_cancelRequested = false;
    QString m_setupError;
};

#endif
