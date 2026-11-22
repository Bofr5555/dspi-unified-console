#include "AppIdentity.h"
#include "Core.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"   // -- WP MDQT1 --
#include "IconProvider.h"
#include "MeasureSpike.h"   // -- WP P5QCAP --
#include "Poller.h"
#include "Registrar.h"
#include "Shell.h"

#include "CoreApi.h"

#include <QApplication>
#include <QCommandLineParser>
#include <QDir>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QSettings>
#include <QStandardPaths>
#include <cstdio>
#include <memory>

#ifdef DSPI_SMOKE
#include "SmokeDriver.h"
#endif

// DSPI_COLOR_SCHEME=light|dark overrides the system's scheme; otherwise the
// app follows the system.
static Qt::ColorScheme colorSchemeOverride() {
    const QString scheme = qEnvironmentVariable("DSPI_COLOR_SCHEME");
    if (scheme == QLatin1String("light")) return Qt::ColorScheme::Light;
    if (scheme == QLatin1String("dark")) return Qt::ColorScheme::Dark;
    return Qt::ColorScheme::Unknown;
}

// -- WP QTCLI -- The command line, the same for both passes in main()
struct CommandLine {
    QCommandLineParser parser;
    // Not addVersionOption(): that prints the application name, which is the
    // settings name (AppIdentity.h), not the name people see
    const QCommandLineOption version{ { QStringLiteral("v"), QStringLiteral("version") },
                                      QApplication::translate("main", "Displays version information.") };
    const QCommandLineOption demo{ QStringLiteral("demo"), QApplication::translate("main", "Open the demo device.") };
    CommandLine() {
        parser.setApplicationDescription(AppIdentity::displayName());
        parser.addHelpOption();
        parser.addOption(version);
        parser.addOption(demo);
        MeasureSpike::addOptions(parser);   // -- WP P5QCAP -- hidden development switches
    }
};

// -- WP QTCLI -- --version and --help (-h, -?, --help-all) need no display:
// they are read before the QApplication is made, so they answer in a terminal
// with no X11 or Wayland. Returns the exit code, or -1 to go on. Qt's own
// options (-platform, -style, ...) are not taken out yet, so each word after
// one dash is one option name here and anything unknown is left to the parse
// after the QApplication.
static int answerInformational(int argc, char *argv[]) {
    QStringList arguments;
    for (int i = 0; i < argc; ++i) arguments.append(QString::fromLocal8Bit(argv[i]));
    CommandLine cl;
    cl.parser.setSingleDashWordOptionMode(QCommandLineParser::ParseAsLongOptions);
    cl.parser.parse(arguments);
#ifndef Q_OS_WIN
    // (On Windows a GUI app's help is a message box, which wants the
    // QApplication; there is always a display there.)
    const bool helpAll = cl.parser.isSet(QStringLiteral("help-all"));
    if (helpAll || cl.parser.isSet(QStringLiteral("help"))) {
        // The help text names the program from the application's arguments;
        // a QCoreApplication needs no display. process() prints it and exits 0.
        QCoreApplication core(argc, argv);
        cl.parser.process(QStringList{ arguments.value(0), helpAll ? QStringLiteral("--help-all") : QStringLiteral("--help") });
        return 0;
    }
#endif
    if (cl.parser.isSet(cl.version)) {
        std::printf("%s %s\n", qPrintable(AppIdentity::displayName()), qPrintable(AppIdentity::version()));
        return 0;
    }
    return -1;
}

// -- WP MDQT1 -- The app's data folders, for every core the app opens and before
// the first device (dspi_core_set_config): the same ones the controllers name
// for the main core, and the speaker layouts' (ROLE0), which none names.
static QJsonObject dataFolders() {
    const QString base = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation);
    const auto folder = [&base](const char *name) { return QDir::toNativeSeparators(base + QLatin1String(name)); };
    QDir().mkpath(base + QStringLiteral("/measurements"));   // as MeasureController: it must exist
    return {
        { QStringLiteral("limitsDir"), folder("/limits") },
        { QStringLiteral("snapshotDir"), folder("/snapshots") },
        { QStringLiteral("measureDir"), folder("/measurements") },
        { QStringLiteral("eventLogDir"), folder("/events") },
        { QStringLiteral("speakerLayoutDir"), folder("/speaker-layouts") },
    };
}

int main(int argc, char *argv[]) {
    // -- WP SWAPFIX -- The settings are an INI file on every system (owner,
    // 2026-11-13): on Windows %APPDATA%\uconsole-for-dspi\uconsole-for-dspi.ini,
    // not the registry (behaviour.md §21). Elsewhere the native format is that
    // INI file already (.conf, which IniFormat would rename). Before the first
    // QSettings.
#ifdef Q_OS_WIN
    QSettings::setDefaultFormat(QSettings::IniFormat);
#endif
    QApplication::setOrganizationName(QString::fromUtf8(AppIdentityNames::settingsOrganisation));
    QApplication::setApplicationName(QString::fromUtf8(AppIdentityNames::settingsApplication));
    QApplication::setApplicationVersion(AppIdentity::version());
    // -- WP QTCLI -- --help and --version answer before the GUI exists
    if (const int answered = answerInformational(argc, argv); answered >= 0) return answered;
    QApplication app(argc, argv);
    QApplication::setApplicationDisplayName(AppIdentity::displayName());
    // The desktop entry (packaging/linux/uconsole-for-dspi.desktop): Wayland's app id
    QGuiApplication::setDesktopFileName(QString::fromUtf8(AppIdentityNames::desktopFileName));
    QQuickStyle::setStyle(QStringLiteral("Basic"));

    CommandLine cl;
    QCommandLineParser &args = cl.parser;
    args.process(app);
    // -- WP P5QCAP -- --audio-inputs, --measure-spike: no window, no core
    if (const int spike = MeasureSpike::runIfAsked(args, app); spike >= 0) return spike;

    FfiCoreSet *set = dspi_core_set_new();
    if (!set) {
        std::fprintf(stderr, "The DSPi core did not start.\n");
        return 1;
    }

    // The QML engine goes first (reset below), then the controllers, the
    // shell, the poller, the device model, the core and last the registry.
    DeviceRegistry registry(set);   // -- WP MDQT1 --
    registry.configure(dataFolders());
    Core core(&registry);
    DeviceModel device(&core, &registry);
    Poller poller(&core, &device);
    auto engine = std::make_unique<QQmlApplicationEngine>();
    Shell shell(engine.get());
    shell.setColorSchemeOverride(colorSchemeOverride());
    QObject controllers;
    Registrar::install(AppContext{ &core, &device, &shell, engine.get(), &poller, &registry }, &controllers);

    engine->addImageProvider(QStringLiteral("icon"), new IconProvider);
    engine->loadFromModule("UConsole", "MainWindow");
    if (engine->rootObjects().isEmpty()) {
        core.shutdown();
        return 1;
    }
    device.start(args.isSet(cl.demo));

#ifdef DSPI_SMOKE
    SmokeDriver driver({ engine.get(), &core, &device, &shell });
    driver.start();
#endif

    const int code = app.exec();
    // Nothing reaches the core after this; the windows go before the engine
    core.shutdown();
    shell.releaseWindows();
    engine.reset();
    return code;
}
