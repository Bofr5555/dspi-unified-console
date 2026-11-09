// BUG1: the K340's own configuration (tests/fixtures/preset/k340-bug1.dspipreset)
// applied to the demo device: what the sidebar rows, their meters and the graph's
// pills show for it, as screenshots, and Reset Channel Names on its I2S slots.
// A launch of its own (run.sh), so the demo device starts and ends as the demo.
#include "SmokeDriver.h"

#include "ConfigFiles.h"
#include "DSPiBridge.h"

#include <QCoreApplication>
#include <QFile>
#include <QQuickItem>
#include <QQuickWindow>
#include <QUrl>
#include <cstdio>
#include <functional>

void SmokeDriver::runK340Labels() {
    section("BUG1: the K340's configuration on the demo device");
    auto *b = m_.bridge;
    auto *main = qobject_cast<QQuickWindow *>(root());
    // The default demo first: no pair map, every output's label derived or its own name
    QStringList demo;
    for (int o = 0; o < b->numOutputChannels(); o++) demo << b->channelLabel(o + 2);
    printf("smoke: BUG1 demo labels: %s\n", qPrintable(demo.join(QStringLiteral(" | "))));
    check("BUG1: the demo's labels", !demo.isEmpty() && demo.first().startsWith(QStringLiteral("Out 1 · ")));
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

    // -- BUG1 option B -- what the rows, the tooltips and the dashboard show for it
    QStringList labels, slotNames;
    for (int o = 0; o < 4; o++) {
        labels << b->channelLabel(o + 2);
        slotNames << b->outputSlotName(o);
    }
    printf("smoke: BUG1 output labels: %s\n", qPrintable(labels.join(QStringLiteral(" | "))));
    check("BUG1: labels from the routing and the filters",
          labels == QStringList({ QStringLiteral("Out 1 · R · Low"), QStringLiteral("Out 2 · R · High"),
                                  QStringLiteral("Out 3 · L · Low"), QStringLiteral("Out 4 · L · High") }));
    check("BUG1: the slots for the tooltips", slotNames == names);
    const QVariantList cards = b->outputCards();
    QStringList shape;
    for (const QVariant &v : cards) {
        const QVariantMap c = v.toMap();
        shape << QStringLiteral("%1+%2 %3|%4").arg(c.value("left").toInt()).arg(c.value("right").toInt())
                     .arg(c.value("leftName").toString(), c.value("rightName").toString());
    }
    printf("smoke: BUG1 cards: %s\n", qPrintable(shape.join(QStringLiteral(", "))));
    check("BUG1: the cards pair by the crossfeed map",
          shape == QStringList({ QStringLiteral("2+0 Woofers L|Woofers R"), QStringLiteral("3+1 Tweeters L|Tweeters R") }));

    const auto grabMain = [&](const QString &file) {
        if (!main) return;
        main->show();
        spin(600);
        main->grabWindow().save(m_out + QStringLiteral("/%1.png").arg(file));
    };
    check("BUG1: the main window", main != nullptr);
    QMetaObject::invokeMethod(root(), "selectOverview");
    grabMain(QStringLiteral("bug1-k340-overview"));
    // Every output row shows its whole label: the sidebar's name column widens for it
    if (main) {
        int shown = 0, cut = 0;
        std::function<void(QQuickItem *)> walk = [&](QQuickItem *t) {
            if (labels.contains(t->property("text").toString()) && t->isVisible()
                && t->metaObject()->indexOfProperty("truncated") >= 0) {
                shown++;
                if (t->property("truncated").toBool()) cut++;
            }
            for (QQuickItem *c : t->childItems()) walk(c);
        };
        walk(main->contentItem());
        printf("smoke: BUG1 label texts shown %d, cut %d\n", shown, cut);
        check("BUG1: the rows show the labels whole", shown >= 4 && cut == 0);
    }
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
    check("BUG1: labels after the reset", b->channelLabel(2) == QStringLiteral("Out 1 · R · Low"));

    // A name the user sets is shown as it is, and a rename starts from it
    b->setChannelName(2, QStringLiteral("Right woofer"));
    spin(400);
    check("BUG1: a user's name wins", b->channelLabel(2) == QStringLiteral("Right woofer")
                                          && b->channelLabel(3) == QStringLiteral("Out 2 · R · High"));
    const QVariantMap first = b->outputCards().value(0).toMap();
    check("BUG1: a named pair keeps its names", first.value("rightName").toString() == QStringLiteral("Right woofer"));
    QMetaObject::invokeMethod(root(), "selectOverview");
    grabMain(QStringLiteral("bug1-k340-user-named"));

    sectionTimes();
    printf("smoke: %d checks, %d failed\n", m_checks, m_failures);
    fflush(stdout);
    QCoreApplication::exit(m_failures ? 1 : 0);
}
