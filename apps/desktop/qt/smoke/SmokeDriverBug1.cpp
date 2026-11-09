// BUG1: the K340's own configuration (tests/fixtures/preset/k340-bug1.dspipreset)
// applied to the demo device: what the sidebar rows, their meters and the graph's
// pills show for it, as screenshots, and Reset Channel Names on its I2S slots.
// A launch of its own (run.sh), so the demo device starts and ends as the demo.
#include "SmokeDriver.h"

#include "ConfigFiles.h"
#include "DSPiBridge.h"

#include <QCoreApplication>
#include <QFile>
#include <QQuickWindow>
#include <QUrl>
#include <cstdio>

void SmokeDriver::runK340Labels() {
    section("BUG1: the K340's configuration on the demo device");
    auto *b = m_.bridge;
    auto *main = qobject_cast<QQuickWindow *>(root());
    const QString doc = m_out + QStringLiteral("/k340-bug1.dspipreset");
    QFile::remove(doc);
    check("BUG1: the fixture", QFile::copy(QStringLiteral(":/smoke/k340-bug1.dspipreset"), doc));
    const QVariantMap r = m_.config->importConfiguration(QUrl::fromLocalFile(doc), false, true);
    printf("smoke: BUG1 import: ok %d %s\n", r.value("ok").toBool(), qPrintable(r.value("error").toString()));
    check("BUG1: import", r.value("ok").toBool());
    spin(400);

    // The K340 as found: USB 1 (L) feeds outputs 3 and 4, USB 2 (R) outputs 1 and 2;
    // outputs 1 and 3 low-pass, 2 and 4 high-pass at 3.5 kHz; slots 1 and 2 are I2S
    const int nIn = b->numInputChannels();
    check("BUG1: routing", b->matrixRouting(0, 2) && b->matrixRouting(0, 3) && b->matrixRouting(1, 0)
                               && b->matrixRouting(1, 1) && !b->matrixRouting(0, 0) && !b->matrixRouting(1, 2));
    check("BUG1: enabled outputs", b->outputEnabled(0) && b->outputEnabled(3) && !b->outputEnabled(4));
    QStringList names;
    for (int o = 0; o < 4; o++) names << b->channelName(o + 2);
    printf("smoke: BUG1 output names: %s (%d inputs)\n", qPrintable(names.join(QStringLiteral(", "))), nIn);
    check("BUG1: the device's names", names == QStringList({ QStringLiteral("I2S 1 L"), QStringLiteral("I2S 1 R"),
                                                             QStringLiteral("I2S 2 L"), QStringLiteral("I2S 2 R") }));

    const auto grabMain = [&](const QString &file) {
        if (!main) return;
        main->show();
        spin(600);
        main->grabWindow().save(m_out + QStringLiteral("/%1.png").arg(file));
    };
    check("BUG1: the main window", main != nullptr);
    QMetaObject::invokeMethod(root(), "selectOverview");
    grabMain(QStringLiteral("bug1-k340-overview"));
    QMetaObject::invokeMethod(root(), "selectOutput", Q_ARG(QVariant, 0));
    grabMain(QStringLiteral("bug1-k340-output1"));

    // Reset Channel Names gives the firmware's defaults, which name an I2S slot I2S
    b->resetChannelNames();
    spin(400);
    names.clear();
    for (int o = 0; o < 9; o++) names << b->channelName(o + 2);
    printf("smoke: BUG1 names after reset: %s\n", qPrintable(names.join(QStringLiteral(", "))));
    check("BUG1: reset names the I2S slots I2S", names == QStringList({
        QStringLiteral("I2S 1 L"), QStringLiteral("I2S 1 R"), QStringLiteral("I2S 2 L"), QStringLiteral("I2S 2 R"),
        QStringLiteral("SPDIF 3 L"), QStringLiteral("SPDIF 3 R"), QStringLiteral("SPDIF 4 L"), QStringLiteral("SPDIF 4 R"),
        QStringLiteral("PDM") }));
    grabMain(QStringLiteral("bug1-k340-after-reset"));

    sectionTimes();
    printf("smoke: %d checks, %d failed\n", m_checks, m_failures);
    fflush(stdout);
    QCoreApplication::exit(m_failures ? 1 : 0);
}
