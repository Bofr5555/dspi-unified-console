// -- WP UB15 -- (UXP B15) Connecting Claude Desktop. "ub15-pictures" grabs
// Settings › AI Agents with no agent paired and the Claude Desktop choice, in
// Modern at 1100×900, in the dark and the light scheme into <out>/ub15/. It
// drives the app through properties only, so the same walk runs on a tree
// without this package (the "before" pictures).
// "ub15-desktop" checks the ClaudeDesktop singleton and the page: where Claude
// Desktop's file is on each system, the extension's manifest, the .mcpb that
// "Install in Claude Desktop…" writes and opens (read back here and by
// Python's zipfile), what the page says when nothing opens it, and "Open
// Config Folder". Nothing is opened for real: the opener is the driver's.
#include "SmokeDriver.h"

#include "AiController.h"
#include "ClaudeDesktopController.h"
#include "LayoutController.h"   // -- FIX016 --
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"

#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QQuickItem>
#include <QQuickWindow>
#include <QStandardPaths>
#include <QTcpServer>
#include <QtEndian>
#include <map>
#include <utility>

namespace {

int freePort() {
    QTcpServer probe;
    return probe.listen(QHostAddress::LocalHost, 0) ? int(probe.serverPort()) : 0;
}

// The window in both schemes, as <out>/ub15/<name>-<scheme>.png
void grabBoth(SmokeDriver &d, QQuickWindow *w, const QString &name) {
    Shell *shell = d.app().shell;
    const Qt::ColorScheme was = shell->colorSchemeOverride();
    for (const auto &[scheme, word] : { std::pair{ Qt::ColorScheme::Dark, "dark" }, std::pair{ Qt::ColorScheme::Light, "light" } }) {
        shell->setColorSchemeOverride(scheme);
        d.spin(300);
        d.grab(w, QStringLiteral("ub15/%1-%2.png").arg(name, QLatin1String(word)));
    }
    shell->setColorSchemeOverride(was);
    d.spin(150);
}

void picturesSection(SmokeDriver &d) {
    auto *ai = Registrar::instance<AiController>();
    Shell *shell = d.app().shell;
    if (!ai || !shell) {
        d.check("ub15-pictures: the controllers exist", false);
        return;
    }
    QDir(QDir(d.outDir()).filePath(QStringLiteral("ub15"))).removeRecursively();
    QDir().mkpath(QDir(d.outDir()).filePath(QStringLiteral("ub15")));
    const bool modernWas = Registrar::instance<LayoutController>()->modern();   // -- FIX016 -- (R1-QT)
    d.switchLayout("modern");
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("store-ub15")));
    ai->setPort(freePort());
    ai->setEnabled(true);
    d.waitFor(10000, [&] { return ai->running() && !ai->busy(); });
    ai->openSettings();
    auto *settings = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check("ub15-pictures: Settings opens", settings != nullptr);
    if (settings) {
        settings->resize(1100, 900);
        d.waitFor(3000, [&] { return d.item(QStringLiteral("AiAgentsPage"), settings->contentItem()) != nullptr; });
        d.spin(500);
        grabBoth(d, settings, QStringLiteral("1-claude-desktop"));
        settings->close();
    }
    ai->setEnabled(false);
    d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
    ai->setPort(AiController::kDefaultPort);
    if (!modernWas) d.switchLayout("classic");   // -- FIX016 -- (R1-QT) the layout as it found it
    d.waitIdle();
}

// The entries of a zip of stored files, by name; empty if it is not one
std::map<QByteArray, QByteArray> unzipStored(const QByteArray &zip) {
    std::map<QByteArray, QByteArray> files;
    const auto u16 = [&](qsizetype at) { return at + 2 <= zip.size() ? qFromLittleEndian<quint16>(zip.constData() + at) : 0; };
    const auto u32 = [&](qsizetype at) { return at + 4 <= zip.size() ? qFromLittleEndian<quint32>(zip.constData() + at) : 0; };
    const qsizetype end = zip.lastIndexOf(QByteArray("PK\x05\x06", 4));
    if (end < 0) return {};
    qsizetype at = u32(end + 16);
    for (int i = 0; i < u16(end + 10); ++i) {
        if (u32(at) != 0x02014b50 || u16(at + 10) != 0) return {};
        const quint32 size = u32(at + 24), local = u32(at + 42);
        const quint16 nameLength = u16(at + 28);
        const QByteArray name = zip.mid(at + 46, nameLength);
        at += 46 + nameLength + u16(at + 30) + u16(at + 32);
        if (u32(local) != 0x04034b50) return {};
        files[name] = zip.mid(local + 30 + u16(local + 26) + u16(local + 28), size);
    }
    return files;
}

void desktopSection(SmokeDriver &d) {
    auto *claude = Registrar::instance<ClaudeDesktopController>();
    auto *ai = Registrar::instance<AiController>();
    Shell *shell = d.app().shell;
    if (!claude || !ai || !shell) {
        d.check("ub15-desktop: the controllers exist", false);
        return;
    }

    // ---- Where Claude Desktop keeps its file
    using C = ClaudeDesktopController;
    d.check("ub15-desktop: macOS: ~/Library/Application Support/Claude",
            C::configFolderFor("darwin", "/Users/u", "", {}) == QDir::toNativeSeparators("/Users/u/Library/Application Support/Claude"));
    d.check("ub15-desktop: Linux: ~/.config/Claude",
            C::configFolderFor("linux", "/home/u", "", {}) == QDir::toNativeSeparators("/home/u/.config/Claude"));
    d.check("ub15-desktop: Windows: %APPDATA%\\Claude",
            C::configFolderFor("win32", "C:/Users/u", "C:/Users/u/AppData/Roaming", {})
                == QDir::toNativeSeparators("C:/Users/u/AppData/Roaming/Claude"));
    d.check("ub15-desktop: Windows, the Store's install: its package's folder when %APPDATA%\\Claude is not there",
            C::configFolderFor("win32", "C:/Users/u", "C:/Users/u/AppData/Roaming", { "C:/L/Packages/Claude_pzs8sxrjxfjjc" })
                == QDir::toNativeSeparators("C:/L/Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude"));
    d.check("ub15-desktop: configFile is the folder's claude_desktop_config.json",
            QDir::fromNativeSeparators(claude->configFile())
                == QDir::fromNativeSeparators(claude->configFolder()) + QStringLiteral("/claude_desktop_config.json"));

    // ---- The Settings search finds the page by the button's and the file's words
    auto *pages = Registrar::instance<SettingsController>();
    d.check("ub15-desktop: the Settings search finds AI Agents for \"extension\" and \"claude_desktop_config\"",
            pages && pages->matches(QStringLiteral("ai"), QStringLiteral("extension"))
                && pages->matches(QStringLiteral("ai"), QStringLiteral("claude_desktop_config")));

    // ---- The manifest: the hand-written configuration's server, for this platform
    const QString program = QDir::toNativeSeparators(QCoreApplication::applicationFilePath());
    const QJsonObject m = C::manifest(program, C::platform());
    bool required = true;
    for (const char *key : { "manifest_version", "name", "version", "description", "author", "server" })
        required = required && m.contains(QLatin1String(key));
    d.check("ub15-desktop: the manifest has the required fields", required && !m.value("author").toObject().value("name").toString().isEmpty());
    const QJsonObject server = m.value("server").toObject(), run = server.value("mcp_config").toObject();
    d.check("ub15-desktop: it starts this program with --mcp, as the configuration does",
            server.value("type").toString() == "binary" && run.value("command").toString() == program
                && run.value("args").toArray() == QJsonArray{ QStringLiteral("--mcp") }
                && QJsonDocument::fromJson(AiController::desktopConfig().toUtf8()).object().value("mcpServers").toObject().value("dspi").toObject().value("command").toString()
                       == program);
    d.check("ub15-desktop: named dspi, as the configuration's server", m.value("name").toString() == "dspi");
    d.check("ub15-desktop: for this platform only", m.value("compatibility").toObject().value("platforms").toArray() == QJsonArray{ C::platform() });

    // ---- Install in Claude Desktop…: written, then opened
    const QString saveDir = QDir(d.outDir()).filePath(QStringLiteral("ub15-extension"));
    QDir(saveDir).removeRecursively();
    claude->setSaveDir(saveDir);
    QList<QUrl> opened;
    bool opens = true;
    claude->setOpener([&](const QUrl &url) {
        opened.append(url);
        return opens;
    });
    const bool modernWas = Registrar::instance<LayoutController>()->modern();   // -- FIX016 -- (R1-QT)
    d.switchLayout("modern");
    ai->setStoreDir(QDir(d.outDir()).filePath(QStringLiteral("store-ub15")));
    ai->setPort(freePort());
    ai->setEnabled(true);
    d.waitFor(10000, [&] { return ai->running() && !ai->busy(); });
    ai->openSettings();
    auto *settings = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check("ub15-desktop: Settings opens", settings != nullptr);
    if (!settings) return;
    settings->resize(1100, 900);
    d.waitFor(3000, [&] { return d.item(QStringLiteral("AiAgentsPage"), settings->contentItem()) != nullptr; });
    d.spin(400);
    QQuickItem *scope = settings->contentItem();
    QQuickItem *install = d.item(QStringLiteral("AiAgents.installDesktop"), scope);
    d.check("ub15-desktop: Install in Claude Desktop… is the Claude Desktop choice's primary button",
            install && install->isVisible() && install->property("primary").toBool()
                && install->property("text").toString() == QStringLiteral("Install in Claude Desktop…"));
    d.check("ub15-desktop: pressing it opens the extension", install && SmokeDriver::press(install) && opened.size() == 1);
    const QString file = QDir(saveDir).filePath(QStringLiteral("uConsole for DSPi.mcpb"));
    d.check("ub15-desktop: ... the file it wrote", !opened.isEmpty() && opened.first() == QUrl::fromLocalFile(file)
                                                     && QDir::fromNativeSeparators(claude->extensionFile()) == file);
    d.check("ub15-desktop: state opened", claude->state() == "opened");
    QQuickItem *said = d.item(QStringLiteral("AiAgents.installOpened"), scope);
    d.spin(100);
    d.check("ub15-desktop: the page says to click Install in Claude Desktop", said && said->isVisible());
    grabBoth(d, settings, QStringLiteral("2-opened"));
    QFile f(file);
    const QByteArray bytes = f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray();
    const auto files = unzipStored(bytes);
    // -- FIX016 -- (REVG F14) and the entry point the manifest names, inside it
    const QByteArray entryName = C::launcherName(C::platform()).toUtf8();
    d.check("ub15-desktop: the .mcpb is a zip of manifest.json, icon.png and its launcher",
            files.size() == 3 && files.count("manifest.json") && files.count("icon.png") && files.count(entryName));
    const QJsonObject written = files.count("manifest.json") ? QJsonDocument::fromJson(files.at("manifest.json")).object() : QJsonObject();
    d.check("ub15-desktop: its manifest is this program's, with the icon",
            written.value("server").toObject() == server && written.value("icon").toString() == "icon.png");
    // -- FIX016 -- (REVG F14) the MCPB rule: server.entry_point is relative to the bundle and in it
    const QString entry = written.value("server").toObject().value("entry_point").toString();
    d.check(QStringLiteral("fix016 F14: the entry point is relative and inside the bundle (%1)").arg(entry),
            !entry.isEmpty() && !QDir::isAbsolutePath(entry) && !entry.contains(QLatin1String("..")) && !entry.contains(QLatin1Char(':'))
                && files.count(entry.toUtf8()) && entry.toUtf8() == entryName);
    const QByteArray launcherText = files.count(entryName) ? files.at(entryName) : QByteArray();
    d.check("fix016 F14: the launcher starts this program with --mcp",
            launcherText.contains(program.toUtf8()) && launcherText.contains("--mcp"));
    {
        const QJsonObject mcp = written.value("server").toObject().value("mcp_config").toObject();
        QByteArray text = files.count("manifest.json") ? files.at("manifest.json") : QByteArray();
        text += launcherText;
        // What is left once the program's own path is taken out: no token, no
        // environment, no user name or home folder
        const QByteArray escaped = QJsonDocument(QJsonArray{ program }).toJson(QJsonDocument::Compact).mid(2).chopped(2);
        text.replace(program.toUtf8(), "").replace(escaped, "");
        const QString user = qEnvironmentVariable("USER", qEnvironmentVariable("USERNAME"));
        const QString home = QDir::homePath();
        d.check("fix016 F14: no token, no environment, no user name in the bundle",
                mcp.value("env").toObject().isEmpty() && !text.contains("token") && !text.contains("Bearer")
                    && !text.toLower().contains("dspi_mcp_") && (user.size() < 3 || !text.contains(user.toUtf8()))
                    && (home.size() < 2 || !text.contains(home.toUtf8())));
    }
    const QImage icon = files.count("icon.png") ? QImage::fromData(files.at("icon.png"), "PNG") : QImage();
    d.check("ub15-desktop: its icon is the app's, 512 px", icon.width() == 512 && icon.height() == 512);
    const QString python = QStandardPaths::findExecutable(QStringLiteral("python3"));
    if (python.isEmpty()) {
        d.notRun("ub15-desktop: Python's zipfile reads the .mcpb", "no python3");
    } else {
        QProcess p;
        p.start(python, { QStringLiteral("-I"), QStringLiteral("-c"),
                          // -- FIX016 -- and the entry point's file, executable
                          QStringLiteral("import json,sys,zipfile; z=zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None; "
                                         "m=json.loads(z.read('manifest.json')); e=m['server']['entry_point']; "
                                         "print(sorted(z.namelist()), m['server']['mcp_config']['args'], e in z.namelist(), "
                                         "oct((z.getinfo(e).external_attr >> 16) & 0o777))"),
                          file });
        p.waitForFinished(10000);
        const QByteArray want = C::platform() == QLatin1String("win32") ? "['icon.png', 'manifest.json', 'server/launch.cmd'] ['--mcp'] True 0o644"
                                                                        : "['icon.png', 'manifest.json', 'server/launch.sh'] ['--mcp'] True 0o755";
        d.check("ub15-desktop: Python's zipfile reads the .mcpb",
                p.exitCode() == 0 && p.readAllStandardOutput().trimmed() == want);
    }

    // ---- Nothing opens it: said, with the file
    opens = false;
    SmokeDriver::press(install);
    d.spin(150);
    QQuickItem *notice = d.item(QStringLiteral("AiAgents.installNotOpened"), scope);
    d.check("ub15-desktop: nothing opened it: the page says so and where the file is",
            claude->state() == "notOpened" && notice && notice->isVisible()
                && notice->property("text").toString().contains(claude->extensionFile()) && !(said && said->isVisible()));
    grabBoth(d, settings, QStringLiteral("3-nothing-opened-it"));
    opened.clear();
    QQuickItem *show = d.item(QStringLiteral("AiAgents.installNotOpened.action"), scope);
    d.check("ub15-desktop: Show File opens the folder that keeps it",
            show && SmokeDriver::press(show) && opened.size() == 1 && opened.first() == QUrl::fromLocalFile(saveDir));

    // ---- The file by hand: where it is, and its folder
    QQuickItem *where = d.item(QStringLiteral("AiAgents.configFile"), scope);
    d.check("ub15-desktop: the page names claude_desktop_config.json's place",
            where && where->isVisible() && where->property("text").toString().contains(claude->configFile()));
    opened.clear();
    QQuickItem *folder = d.item(QStringLiteral("AiAgents.openConfigFolder"), scope);
    d.check("ub15-desktop: Open Config Folder opens it (made if Claude Desktop has not yet)",
            folder && SmokeDriver::press(folder) && opened.size() == 1
                && opened.first() == QUrl::fromLocalFile(QDir::fromNativeSeparators(claude->configFolder()))
                && QFileInfo(QDir::fromNativeSeparators(claude->configFolder())).isDir());

    // ---- Only on the Claude Desktop choice
    QQuickItem *segments = d.item(QStringLiteral("AiAgents.segments"), scope);
    if (segments) QMetaObject::invokeMethod(segments, "chosen", Q_ARG(int, 1));
    d.spin(150);
    d.check("ub15-desktop: Claude Code's choice has neither", segments && install && !install->isVisible() && folder && !folder->isVisible()
                                                              && notice && !notice->isVisible());
    if (segments) QMetaObject::invokeMethod(segments, "chosen", Q_ARG(int, 0));

    // As it was
    claude->setOpener({});
    claude->setSaveDir({});
    settings->close();
    ai->setEnabled(false);
    d.waitFor(10000, [&] { return !ai->running() && !ai->busy(); });
    ai->setPort(AiController::kDefaultPort);
    if (!modernWas) d.switchLayout("classic");   // -- FIX016 -- (R1-QT) the layout as it found it
    d.waitIdle();
}

const SmokeDriver::Section pictures("ub15-pictures", 999, &picturesSection);
const SmokeDriver::Section desktop("ub15-desktop", 1000, &desktopSection);

} // namespace
