#include "AppIdentity.h"
#include "Core.h"
#include "DeviceModel.h"
#include "IconProvider.h"
#include "Poller.h"
#include "Registrar.h"
#include "Shell.h"

#include "CoreApi.h"

#include <QApplication>
#include <QCommandLineParser>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
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

int main(int argc, char *argv[]) {
    QApplication::setOrganizationName(QString::fromUtf8(AppIdentityNames::settingsOrganisation));
    QApplication::setApplicationName(QString::fromUtf8(AppIdentityNames::settingsApplication));
    QApplication::setApplicationVersion(AppIdentity::version());
    QApplication app(argc, argv);
    QApplication::setApplicationDisplayName(AppIdentity::displayName());
    // The desktop entry (packaging/linux/uconsole-for-dspi.desktop): Wayland's app id
    QGuiApplication::setDesktopFileName(QString::fromUtf8(AppIdentityNames::desktopFileName));
    QQuickStyle::setStyle(QStringLiteral("Basic"));

    QCommandLineParser args;
    args.setApplicationDescription(AppIdentity::displayName());
    args.addHelpOption();
    // Not addVersionOption(): that prints the application name, which is the
    // settings name (AppIdentity.h), not the name people see
    const QCommandLineOption version({ QStringLiteral("v"), QStringLiteral("version") },
                                     QApplication::translate("main", "Displays version information."));
    args.addOption(version);
    const QCommandLineOption demo(QStringLiteral("demo"), QApplication::translate("main", "Open the demo device."));
    args.addOption(demo);
    args.process(app);
    if (args.isSet(version)) {
        std::printf("%s %s\n", qPrintable(AppIdentity::displayName()), qPrintable(AppIdentity::version()));
        return 0;
    }

    FfiCore *ffi = dspi_core_new();
    if (!ffi) {
        std::fprintf(stderr, "The DSPi core did not start.\n");
        return 1;
    }

    // The QML engine goes first (reset below), then the controllers, the
    // shell, the poller, the device model and last the core.
    Core core(ffi);
    DeviceModel device(&core);
    Poller poller(&core, &device);
    auto engine = std::make_unique<QQmlApplicationEngine>();
    Shell shell(engine.get());
    shell.setColorSchemeOverride(colorSchemeOverride());
    QObject controllers;
    Registrar::install(AppContext{ &core, &device, &shell, engine.get(), &poller }, &controllers);

    engine->addImageProvider(QStringLiteral("icon"), new IconProvider);
    engine->loadFromModule("UConsole", "MainWindow");
    if (engine->rootObjects().isEmpty()) {
        core.shutdown();
        return 1;
    }
    device.start(args.isSet(demo));

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
