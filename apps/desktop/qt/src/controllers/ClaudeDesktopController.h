#ifndef CLAUDEDESKTOPCONTROLLER_H
#define CLAUDEDESKTOPCONTROLLER_H

#include <QByteArray>
#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QStringList>
#include <QUrl>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP UB15 -- (UXP B15) Claude Desktop in one click: Settings › AI Agents'
// Claude Desktop choice. QML sees it as the singleton `ClaudeDesktop`.
//
// - install() writes a Claude Desktop extension (an .mcpb: a zip of a manifest
//   and an icon) that starts this app's own relay, `<program> --mcp`, as the
//   hand-written configuration does, and opens it: Claude Desktop shows its own
//   install sheet. The program is this computer's, so the extension is made
//   here and not shipped in the packages.
// - For a host without extensions, or a hand-written configuration: where
//   claude_desktop_config.json is (`configFile`) and openConfigFolder().
class ClaudeDesktopController : public QObject {
    Q_OBJECT
    QML_NAMED_ELEMENT(ClaudeDesktop)
    QML_SINGLETON
    // Where Claude Desktop keeps claude_desktop_config.json, and the file
    Q_PROPERTY(QString configFolder READ configFolder CONSTANT)
    Q_PROPERTY(QString configFile READ configFile CONSTANT)
    // How the last install() went: "" (none yet), "opened" (Claude Desktop, or
    // what opens .mcpb files here, was asked to open it), "notOpened" (nothing
    // opened it: the file stays in `extensionFile`), "failed" (not written:
    // `error`)
    Q_PROPERTY(QString state READ state NOTIFY stateChanged)
    Q_PROPERTY(QString extensionFile READ extensionFile NOTIFY stateChanged)
    Q_PROPERTY(QString error READ error NOTIFY stateChanged)
public:
    ClaudeDesktopController(const AppContext &ctx, QObject *parent);
    static ClaudeDesktopController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<ClaudeDesktopController>(); }

    QString configFolder() const { return m_configFolder; }
    QString configFile() const;
    QString state() const { return m_state; }
    QString extensionFile() const { return m_extensionFile; }
    QString error() const { return m_error; }

    Q_INVOKABLE void install();
    Q_INVOKABLE void openConfigFolder();
    // The folder that keeps the extension, after "notOpened"
    Q_INVOKABLE void showExtensionFile();

    // The extension's manifest for `program` (started with --mcp) on `platform`
    // ("darwin", "win32", "linux": Node's names, as Claude Desktop's)
    static QJsonObject manifest(const QString &program, const QString &platform);
    // The .mcpb: manifest.json and icon.png (`icon`, a PNG) in a zip
    static QByteArray bundle(const QJsonObject &manifest, const QByteArray &icon);
    // -- FIX016 -- (REVG F14) The bundle's entry point, relative to it
    // ("server/launch.sh", "server/launch.cmd" on win32), and its text: it
    // starts `program` with --mcp
    static QString launcherName(const QString &platform);
    static QByteArray launcher(const QString &program, const QString &platform);
    // Claude Desktop's folder on `platform`: under the home folder on macOS and
    // Linux, under %APPDATA% on Windows; the Microsoft Store's install keeps it
    // in its package's folder (one of `packaged`, %LOCALAPPDATA%\Packages\Claude_*)
    // when %APPDATA%'s does not exist
    static QString configFolderFor(const QString &platform, const QString &home, const QString &appData,
                                   const QStringList &packaged);
    static QString platform();

    // For the smoke driver: what opens a file (QDesktopServices::openUrl), and
    // where the extension is written
    void setOpener(std::function<bool(const QUrl &)> opener) { m_opener = std::move(opener); }
    void setSaveDir(const QString &dir) { m_saveDir = dir; }

signals:
    void stateChanged();

private:
    void setState(const QString &state, const QString &error = {});

    QString m_configFolder;
    QString m_saveDir;
    QString m_state;
    QString m_extensionFile;
    QString m_error;
    std::function<bool(const QUrl &)> m_opener;
};

#endif // CLAUDEDESKTOPCONTROLLER_H
