#include "ClaudeDesktopController.h"

#include "AppIdentity.h"

#include <QBuffer>
#include <QCoreApplication>
#include <QDesktopServices>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QSaveFile>
#include <QStandardPaths>
#include <QtEndian>
#include <array>
#include <utility>
#include <vector>

static const Registrar::Controller<ClaudeDesktopController> claudeDesktopController;

namespace {


quint32 crc32(const QByteArray &data) {
    static const std::array<quint32, 256> table = [] {
        std::array<quint32, 256> t{};
        for (quint32 i = 0; i < 256; ++i) {
            quint32 c = i;
            for (int k = 0; k < 8; ++k) c = (c & 1) ? 0xEDB88320u ^ (c >> 1) : c >> 1;
            t[i] = c;
        }
        return t;
    }();
    quint32 c = 0xFFFFFFFFu;
    for (const char b : data) c = table[(c ^ quint8(b)) & 0xFF] ^ (c >> 8);
    return c ^ 0xFFFFFFFFu;
}

void le16(QByteArray &out, quint16 v) {
    char b[2];
    qToLittleEndian(v, b);
    out.append(b, 2);
}

void le32(QByteArray &out, quint32 v) {
    char b[4];
    qToLittleEndian(v, b);
    out.append(b, 4);
}

// A zip of `files` (name, contents), stored as they are: small, and every
// unzipper reads it. Dated 1980-01-01, so the same files make the same bytes.
// -- FIX016 -- A name ending in .sh is marked executable (Unix mode 0755 in the
// central directory, as Info-ZIP writes it), the rest 0644.
QByteArray zip(const std::vector<std::pair<QByteArray, QByteArray>> &files) {
    constexpr quint16 version = 20, date = (1 << 5) | 1;
    constexpr quint16 madeOnUnix = (3 << 8) | version;
    QByteArray out, central;
    for (const auto &[name, data] : files) {
        const quint32 offset = quint32(out.size()), crc = crc32(data), size = quint32(data.size());
        le32(out, 0x04034b50);
        for (const quint16 v : { version, quint16(0), quint16(0), quint16(0), date }) le16(out, v);
        for (const quint32 v : { crc, size, size }) le32(out, v);
        le16(out, quint16(name.size()));
        le16(out, 0);
        out += name;
        out += data;
        le32(central, 0x02014b50);
        for (const quint16 v : { madeOnUnix, version, quint16(0), quint16(0), quint16(0), date }) le16(central, v);
        for (const quint32 v : { crc, size, size }) le32(central, v);
        for (const quint16 v : { quint16(name.size()), quint16(0), quint16(0), quint16(0), quint16(0) }) le16(central, v);
        le32(central, (name.endsWith(".sh") ? 0100755u : 0100644u) << 16);   // -- FIX016 --
        le32(central, offset);
        central += name;
    }
    const quint32 at = quint32(out.size());
    out += central;
    le32(out, 0x06054b50);
    for (const quint16 v : { quint16(0), quint16(0), quint16(files.size()), quint16(files.size()) }) le16(out, v);
    le32(out, quint32(central.size()));
    le32(out, at);
    le16(out, 0);
    return out;
}

// The app's icon at 512 px, the size Claude Desktop recommends (mcpb validate)
QByteArray iconPng() {
    const QImage icon(QStringLiteral(":/ub15/dspi-icon-rounded-1024.png"));
    if (icon.isNull()) return {};
    QByteArray png;
    QBuffer buffer(&png);
    buffer.open(QIODevice::WriteOnly);
    icon.scaled(512, 512, Qt::KeepAspectRatio, Qt::SmoothTransformation).save(&buffer, "PNG");
    return png;
}

} // namespace

ClaudeDesktopController::ClaudeDesktopController(const AppContext &, QObject *parent) : QObject(parent) {
    QStringList packaged;
    const QString local = qEnvironmentVariable("LOCALAPPDATA");
    if (platform() == QLatin1String("win32") && !local.isEmpty()) {
        const QDir packages(local + QStringLiteral("/Packages"));
        for (const QString &p : packages.entryList({ QStringLiteral("Claude_*") }, QDir::Dirs | QDir::NoDotAndDotDot))
            packaged.append(packages.filePath(p));
    }
    m_configFolder = configFolderFor(platform(), QDir::homePath(), qEnvironmentVariable("APPDATA"), packaged);
}

QString ClaudeDesktopController::platform() {
#if defined(Q_OS_MACOS)
    return QStringLiteral("darwin");
#elif defined(Q_OS_WIN)
    return QStringLiteral("win32");
#else
    return QStringLiteral("linux");
#endif
}

QString ClaudeDesktopController::configFolderFor(const QString &platform, const QString &home, const QString &appData,
                                                 const QStringList &packaged) {
    QString folder;
    if (platform == QLatin1String("darwin")) {
        folder = home + QStringLiteral("/Library/Application Support/Claude");
    } else if (platform == QLatin1String("win32")) {
        folder = QDir::fromNativeSeparators(appData) + QStringLiteral("/Claude");
        if (!QFileInfo(folder).isDir() && !packaged.isEmpty())
            folder = QDir::fromNativeSeparators(packaged.first()) + QStringLiteral("/LocalCache/Roaming/Claude");
    } else {
        folder = home + QStringLiteral("/.config/Claude");
    }
    return QDir::toNativeSeparators(folder);
}

QString ClaudeDesktopController::configFile() const {
    return QDir::toNativeSeparators(QDir::fromNativeSeparators(m_configFolder) + QStringLiteral("/claude_desktop_config.json"));
}

// -- FIX016 -- (REVG F14) The bundle's own entry point, inside it as the MCPB
// manifest asks (server.entry_point is relative to the bundle): a launcher of
// a few lines that starts this computer's copy of the app with --mcp. The app
// is not in the bundle (it is installed, and an AppImage moves with updates).
QString ClaudeDesktopController::launcherName(const QString &platform) {
    return platform == QLatin1String("win32") ? QStringLiteral("server/launch.cmd") : QStringLiteral("server/launch.sh");
}

QByteArray ClaudeDesktopController::launcher(const QString &program, const QString &platform) {
    if (platform == QLatin1String("win32"))
        return QByteArrayLiteral("@echo off\r\nrem ") + AppIdentity::displayName().toUtf8()
               + QByteArrayLiteral("'s MCP relay: this computer's copy of the app\r\n\"") + program.toUtf8()
               + QByteArrayLiteral("\" --mcp %*\r\n");
    QString quoted = program;
    quoted.replace(QLatin1Char('\''), QStringLiteral("'\\''"));
    return QByteArrayLiteral("#!/bin/sh\n# ") + AppIdentity::displayName().toUtf8()
           + QByteArrayLiteral("'s MCP relay: this computer's copy of the app\nexec '") + quoted.toUtf8()
           + QByteArrayLiteral("' --mcp \"$@\"\n");
}

QJsonObject ClaudeDesktopController::manifest(const QString &program, const QString &platform) {
    // The same server as the hand-written configuration ("dspi": program --mcp),
    // for this computer only: the program is where it is installed here.
    // -- FIX016 -- (F14) mcp_config runs the program itself, as the hand-written
    // configuration does (no launcher's execute bit or a .cmd's shell needed);
    // entry_point is the bundle's launcher of the same command
    const QJsonObject run{ { QStringLiteral("command"), program },
                           { QStringLiteral("args"), QJsonArray{ QStringLiteral("--mcp") } },
                           { QStringLiteral("env"), QJsonObject() } };
    const QJsonObject author{ { QStringLiteral("name"), AppIdentity::displayName() }, { QStringLiteral("url"), AppIdentity::projectUrl() } };
    return {
        { QStringLiteral("manifest_version"), QStringLiteral("0.3") },
        { QStringLiteral("name"), QStringLiteral("dspi") },
        { QStringLiteral("display_name"), AppIdentity::displayName() },
        { QStringLiteral("version"), AppIdentity::version() },
        { QStringLiteral("description"),
          tr("Lets Claude read and adjust your DSPi devices through %1. The app asks you before risky changes, and your limits always hold.")
              .arg(AppIdentity::displayName()) },
        { QStringLiteral("author"), author },
        { QStringLiteral("homepage"), AppIdentity::projectUrl() },
        { QStringLiteral("icon"), QStringLiteral("icon.png") },
        { QStringLiteral("server"), QJsonObject{ { QStringLiteral("type"), QStringLiteral("binary") },
                                                 { QStringLiteral("entry_point"), launcherName(platform) },   // -- FIX016 -- relative
                                                 { QStringLiteral("mcp_config"), run } } },
        { QStringLiteral("tools_generated"), true },
        { QStringLiteral("compatibility"), QJsonObject{ { QStringLiteral("platforms"), QJsonArray{ platform } } } },
    };
}

QByteArray ClaudeDesktopController::bundle(const QJsonObject &manifest, const QByteArray &icon) {
    std::vector<std::pair<QByteArray, QByteArray>> files{ { "manifest.json", QJsonDocument(manifest).toJson() } };
    if (!icon.isEmpty()) files.emplace_back("icon.png", icon);
    // -- FIX016 -- (F14) the entry point the manifest names, in the bundle
    const QJsonObject server = manifest.value(QLatin1String("server")).toObject();
    const QString entry = server.value(QLatin1String("entry_point")).toString();
    const QString platform = manifest.value(QLatin1String("compatibility")).toObject().value(QLatin1String("platforms")).toArray().first().toString();
    if (!entry.isEmpty())
        files.emplace_back(entry.toUtf8(), launcher(server.value(QLatin1String("mcp_config")).toObject().value(QLatin1String("command")).toString(), platform));
    return zip(files);
}

void ClaudeDesktopController::setState(const QString &state, const QString &error) {
    m_state = state;
    m_error = error;
    emit stateChanged();
}

void ClaudeDesktopController::install() {
    const QByteArray icon = iconPng();
    QJsonObject m = manifest(AppIdentity::programPath(), platform());   // -- FIX016 -- (F14) one helper
    if (icon.isEmpty()) m.remove(QStringLiteral("icon"));
    const QString dir = m_saveDir.isEmpty() ? QStandardPaths::writableLocation(QStandardPaths::AppLocalDataLocation) : m_saveDir;
    QDir().mkpath(dir);
    const QString path = QDir(dir).filePath(AppIdentity::displayName() + QStringLiteral(".mcpb"));
    QSaveFile f(path);
    if (!f.open(QIODevice::WriteOnly) || f.write(bundle(m, icon)) < 0 || !f.commit()) {
        m_extensionFile.clear();
        setState(QStringLiteral("failed"), f.errorString());
        return;
    }
    m_extensionFile = QDir::toNativeSeparators(path);
    const QUrl url = QUrl::fromLocalFile(path);
    const bool opened = m_opener ? m_opener(url) : QDesktopServices::openUrl(url);
    setState(opened ? QStringLiteral("opened") : QStringLiteral("notOpened"));
}

void ClaudeDesktopController::openConfigFolder() {
    // Claude Desktop makes it when it first starts; made here for a file
    // written before that
    QDir().mkpath(QDir::fromNativeSeparators(m_configFolder));
    const QUrl url = QUrl::fromLocalFile(QDir::fromNativeSeparators(m_configFolder));
    if (m_opener) m_opener(url);
    else QDesktopServices::openUrl(url);
}

void ClaudeDesktopController::showExtensionFile() {
    if (m_extensionFile.isEmpty()) return;
    const QUrl url = QUrl::fromLocalFile(QFileInfo(QDir::fromNativeSeparators(m_extensionFile)).absolutePath());
    if (m_opener) m_opener(url);
    else QDesktopServices::openUrl(url);
}
