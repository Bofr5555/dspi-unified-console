#include "AppIdentity.h"
#include "Core.h"
#include "DeviceModel.h"
#include "IconProvider.h"
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
    QQuickStyle::setStyle(QStringLiteral("Basic"));

    QCommandLineParser args;
    args.setApplicationDescription(AppIdentity::displayName());
    args.addHelpOption();
    args.addVersionOption();
    const QCommandLineOption demo(QStringLiteral("demo"), QApplication::translate("main", "Open the demo device."));
    args.addOption(demo);
    args.process(app);

    FfiCore *ffi = dspi_core_new();
    if (!ffi) {
        std::fprintf(stderr, "The DSPi core did not start.\n");
        return 1;
    }

    // The QML engine goes first (reset below), then the controllers, the
    // shell, the device model and last the core.
    Core core(ffi);
    DeviceModel device(&core);
    auto engine = std::make_unique<QQmlApplicationEngine>();
    Shell shell(engine.get());
    shell.setColorSchemeOverride(colorSchemeOverride());
    QObject controllers;
    Registrar::install(AppContext{ &core, &device, &shell, engine.get() }, &controllers);

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
