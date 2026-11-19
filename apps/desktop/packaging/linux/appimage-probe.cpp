// -- WP APPIMG -- A test probe for the AppImage, never shipped: test-appimage.sh
// builds it against the release's Qt (Containerfile.ubuntu's /opt/qt) and
// LD_PRELOADs it into the extracted AppImage, offscreen, on the demo device.
// Once the main window is up it opens Help > About (Shell's "help.about") and
// prints, one "probe: key=value" line each, what the release binary itself
// shows and loaded:
//   version, licence, gpl     the About page's version line and licence section
//   notices, noticesDir       AboutController's notice files and their folder
//   qtpath.<name>             QLibraryInfo's paths (qt.conf in usr/bin)
//   importpath                the QML engine's import paths
//   lib                       every Qt library or plugin mapped (/proc/self/maps)
// then saves the About window as $PROBE_PNG (when set) and quits with 0, or
// with 3 when it found no main window or no About page within 10 s.
#include <QCoreApplication>
#include <QFile>
#include <QGuiApplication>
#include <QLibraryInfo>
#include <QQmlEngine>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSet>
#include <QTimer>
#include <cstdio>

static void say(const char *key, const QString &value) {
    std::printf("probe: %s=%s\n", key, qPrintable(QString(value).replace(QLatin1Char('\n'), QLatin1Char(' '))));
    std::fflush(stdout);
}

// The visual tree, as the smoke driver searches it (a Loader's page is a child item)
static QQuickItem *findIn(QQuickItem *scope, const QString &name) {
    if (scope->objectName() == name) return scope;
    for (QQuickItem *child : scope->childItems())
        if (QQuickItem *found = findIn(child, name)) return found;
    return nullptr;
}

static QQuickItem *findItem(const QString &name) {
    for (QWindow *w : QGuiApplication::allWindows()) {
        auto *qw = qobject_cast<QQuickWindow *>(w);
        if (!qw) continue;
        if (QQuickItem *item = findIn(qw->contentItem(), name)) return item;
    }
    return nullptr;
}

static void finish(int tries) {
    QQuickItem *version = findItem(QStringLiteral("About.version"));
    if (!version && tries > 0) {
        QTimer::singleShot(500, qApp, [tries] { finish(tries - 1); });
        return;
    }
    if (!version) {
        QStringList windows;
        for (QWindow *w : QGuiApplication::allWindows()) windows << w->title() + (w->isVisible() ? "(shown)" : "");
        say("error", QStringLiteral("no About page after help.about; windows: ") + windows.join(QStringLiteral(", ")));
        QCoreApplication::exit(3);
        return;
    }
    say("version", version->property("text").toString());
    for (const char *n : { "About.licenceStatement", "About.licenceGpl", "About.licenceNotices" }) {
        QQuickItem *item = findItem(QString::fromLatin1(n));
        say(n, item ? item->property("text").toString() : QStringLiteral("<missing>"));
    }
    QQmlEngine *engine = qmlEngine(version);
    if (QObject *about = engine->singletonInstance<QObject *>("UConsole", "AboutController")) {
        say("noticesDir", about->property("noticesDir").toString());
        say("notices", QString::number(about->property("notices").toList().size()));
    }
    say("importpath", engine->importPathList().join(QLatin1Char(':')));
    const QByteArray png = qgetenv("PROBE_PNG");
    if (!png.isEmpty()) version->window()->grabWindow().save(QString::fromLocal8Bit(png));

    QFile maps(QStringLiteral("/proc/self/maps"));
    QSet<QString> seen;
    if (maps.open(QIODevice::ReadOnly)) {
        for (const QByteArray &line : maps.readAll().split('\n')) {
            const int slash = line.indexOf('/');
            if (slash < 0) continue;
            const QString path = QString::fromLocal8Bit(line.mid(slash));
            if (!path.contains(QLatin1String(".so"))) continue;
            if (!path.contains(QLatin1String("Qt6")) && !path.contains(QLatin1String("/plugins/"))
                && !path.contains(QLatin1String("/qml/")))
                continue;
            if (!seen.contains(path)) say("lib", path);
            seen.insert(path);
        }
    }
    QCoreApplication::exit(0);
}

static void start() {
    say("qtpath.plugins", QLibraryInfo::path(QLibraryInfo::PluginsPath));
    say("qtpath.qml", QLibraryInfo::path(QLibraryInfo::QmlImportsPath));
    say("qtpath.translations", QLibraryInfo::path(QLibraryInfo::TranslationsPath));
    say("qtpath.libraries", QLibraryInfo::path(QLibraryInfo::LibrariesPath));
    // The main window and the demo device come up after the event loop starts
    QTimer::singleShot(6000, qApp, [] {
        QObject *shell = nullptr;
        for (QWindow *w : QGuiApplication::topLevelWindows()) {
            auto *qw = qobject_cast<QQuickWindow *>(w);
            if (!qw || !qmlEngine(qw)) continue;
            shell = qmlEngine(qw)->singletonInstance<QObject *>("UConsole", "Shell");
            if (shell) break;
        }
        if (!shell) {
            say("error", QStringLiteral("no main window"));
            QCoreApplication::exit(3);
            return;
        }
        QMetaObject::invokeMethod(shell, "trigger", Q_ARG(QString, QStringLiteral("help.about")));
        finish(20);
    });
}
Q_COREAPP_STARTUP_FUNCTION(start)
