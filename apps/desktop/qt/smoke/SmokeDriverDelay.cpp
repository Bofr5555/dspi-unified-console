// WP SL1: the delay slider and the delay wheels send and commit whole ms, as
// the macOS console (release/v1.1.6) rounds them, from a fractional start
// too; a fractional delay set elsewhere stays shown until the user drags, and
// a typed one is kept as typed.
#include "SmokeDriver.h"

#include "ConfigFiles.h"   // -- WP DL3 --
#include "DSPiBridge.h"

#include <QCoreApplication>
#include <QFile>   // -- WP DL3 --
#include <QMouseEvent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QWheelEvent>
#include <cmath>
#include <cstdio>

namespace {
QQuickItem *findItem(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = findItem(c, name)) return found;
    return nullptr;
}

QQuickItem *textInput(QQuickItem *from) {
    if (from->inherits("QQuickTextInput")) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = textInput(c)) return found;
    return nullptr;
}

void mouse(QQuickWindow *w, QEvent::Type type, const QPointF &at, Qt::MouseButtons held) {
    const Qt::MouseButton button = type == QEvent::MouseMove ? Qt::NoButton : Qt::LeftButton;
    QMouseEvent e(type, at, at, QPointF(w->mapToGlobal(at.toPoint())), button, held, Qt::NoModifier);
    QCoreApplication::sendEvent(w, &e);
}

void wheelOn(QQuickItem *item, int delta, Qt::KeyboardModifiers mods) {
    QQuickWindow *w = item->window();
    const QPointF at = item->mapToScene(QPointF(item->width() / 2, item->height() / 2));
    QWheelEvent e(at, QPointF(w->mapToGlobal(at.toPoint())), QPoint(), QPoint(0, delta), Qt::NoButton, mods, Qt::NoScrollPhase, false);
    QCoreApplication::sendEvent(w, &e);
}

bool whole(float ms) { return std::fabs(ms - std::round(ms)) < 1e-6f; }
bool near(double a, double b, double tol) { return std::fabs(a - b) <= tol; }   // -- WP DL3 --
}

void SmokeDriver::testDelaySliders() {
    section("delay sliders: whole ms (SL1)");
    auto *b = m_.bridge;
    check("SL1: the demo device is writable", waitFor(2000, [b] { return b->connected() && b->writable(); }));
    auto *main = qobject_cast<QQuickWindow *>(root());
    check("SL1: the main window", main != nullptr);
    if (!main) return;
    main->show();
    const float before = b->outputDelayMS(0);
    QMetaObject::invokeMethod(root(), "selectOutput", Q_ARG(QVariant, 0));
    spin(300);
    QQuickItem *page = nullptr;
    for (QQuickItem *i : root()->findChildren<QQuickItem *>())
        if (QString::fromLatin1(i->metaObject()->className()).startsWith(QLatin1String("ChannelSettingsView")) && i->isVisible()) page = i;
    QQuickItem *slider = page ? findItem(page, QStringLiteral("delaySectionSlider")) : nullptr;
    QQuickItem *field = page ? findItem(page, QStringLiteral("delaySectionField")) : nullptr;
    QQuickItem *input = field ? textInput(field) : nullptr;
    check("SL1: the output page, its delay slider and field", page && slider && field && input);
    if (!page || !slider || !field || !input) return;
    const auto shows = [&](float ms) {
        return std::fabs(page->property("delayMS").toDouble() - ms) < 1e-4 && std::fabs(slider->property("value").toDouble() - ms) < 1e-4;
    };

    // A fractional delay from elsewhere (a file, another client, typing) is shown as it is
    b->setOutputDelay(0, 0.25f);
    check("SL1: a fractional delay is kept", waitFor(1000, [&] { return std::fabs(b->outputDelayMS(0) - 0.25f) < 1e-6f; }));
    check("SL1: and the page shows it", waitFor(1000, [&] { return shows(0.25f); }));
    check("SL1: the field reads 0.25", input->property("text").toString() == QLatin1String("0.25"));

    // A drag from it: every value sent while dragging and the committed one are whole ms
    const double pad = slider->property("leftPadding").toDouble();
    const double track = slider->property("availableWidth").toDouble();
    const double y = slider->height() / 2;
    const auto at = [&](double pos) { return slider->mapToScene(QPointF(pad + 9 + pos * (track - 18), y)); };
    const double start = slider->property("visualPosition").toDouble();
    mouse(main, QEvent::MouseMove, at(start), Qt::NoButton);
    mouse(main, QEvent::MouseButtonPress, at(start), Qt::LeftButton);
    spin(40);
    int samples = 0, fractional = 0;
    for (int k = 1; k <= 12; k++) {
        // 0.37 of a step per move, so most moves end between two whole ms
        mouse(main, QEvent::MouseMove, at(start + k * 0.37 / b->maxDelayMs()), Qt::LeftButton);
        spin(45);
        samples++;
        if (!whole(b->outputDelayMS(0))) {
            fractional++;
            printf("smoke: SL1 sent %.4f ms while dragging\n", double(b->outputDelayMS(0)));
        }
    }
    check("SL1: every value sent while dragging is whole ms", samples > 0 && fractional == 0);
    check("SL1: the drag moved the delay", b->outputDelayMS(0) >= 1.0f);
    mouse(main, QEvent::MouseButtonRelease, at(start + 12 * 0.37 / b->maxDelayMs()), Qt::NoButton);
    spin(100);
    const float committed = b->outputDelayMS(0);
    printf("smoke: SL1 drag from 0.25 committed %.4f ms\n", double(committed));
    check("SL1: the release commits whole ms", committed >= 1.0f && whole(committed));
    check("SL1: the slider and page snap to it", waitFor(1000, [&] { return shows(committed); }));

    // Typing keeps the fraction
    input->forceActiveFocus();
    input->setProperty("text", QStringLiteral("2.25"));
    QMetaObject::invokeMethod(input, "editingFinished");
    spin(100);
    check("SL1: a typed 2.25 ms is kept", std::fabs(b->outputDelayMS(0) - 2.25f) < 1e-6f);
    input->setFocus(false);
    main->contentItem()->forceActiveFocus();

    // Ctrl+wheel on the field from a fractional value: the next whole ms
    b->setOutputDelay(0, 0.25f);
    spin(100);
    wheelOn(field, 120, Qt::ControlModifier);
    check("SL1: Ctrl+wheel up from 0.25 ms gives 1 ms", waitFor(1000, [&] { return b->outputDelayMS(0) == 1.0f; }));
    b->setOutputDelay(0, 1.5f);
    spin(100);
    wheelOn(field, -120, Qt::ControlModifier);
    check("SL1: Ctrl+wheel down from 1.5 ms gives 1 ms", waitFor(1000, [&] { return b->outputDelayMS(0) == 1.0f; }));
    wheelOn(field, 120, Qt::ControlModifier);
    check("SL1: and from a whole value one whole ms", waitFor(1000, [&] { return b->outputDelayMS(0) == 2.0f; }));

    // The Matrix Mixer's delay field: the plain wheel, the same grid
    QQuickWindow *matrix = root()->findChild<QQuickWindow *>(QStringLiteral("matrixWindow"));
    if (matrix) {
        QMetaObject::invokeMethod(root(), "openToolWindow", Q_ARG(QVariant, QStringLiteral("matrix")));
        spin(400);
        b->setOutputDelay(0, 0.5f);
        spin(200);
        QQuickItem *cell = findItem(matrix->contentItem(), QStringLiteral("matrixDelayField0"));
        QQuickItem *cellInput = cell ? textInput(cell) : nullptr;
        check("SL1: the Matrix delay field", cell && cellInput);
        if (cell && cellInput) {
            check("SL1: the Matrix shows 0.5 ms", waitFor(1000, [&] { return cellInput->property("text").toString() == QLatin1String("0.50"); }));
            wheelOn(cell, -120, Qt::NoModifier);
            check("SL1: the Matrix wheel down from 0.5 ms gives 0 ms", waitFor(1000, [&] { return b->outputDelayMS(0) == 0.0f; }));
            b->setOutputDelay(0, 0.5f);
            spin(200);
            wheelOn(cell, 120, Qt::NoModifier);
            check("SL1: the Matrix wheel up from 0.5 ms gives 1 ms", waitFor(1000, [&] { return b->outputDelayMS(0) == 1.0f; }));
        }
        matrix->setVisible(false);
        spin(100);
    } else {
        check("SL1: the Matrix window", false);
    }

    b->setOutputDelay(0, before);
    QMetaObject::invokeMethod(root(), "selectOverview");
    spin(100);
}

// -- WP DL3 -- A delay past the device's line is refused in the core's words and
// shown on the banner; a pasted one is held to the line; an imported one is
// shortened and named in the report.
void SmokeDriver::testDelayRefusal() {
    section("delay refusal and the line (DL3)");
    auto *b = m_.bridge;
    check("DL3: the demo device is writable", waitFor(2000, [b] { return b->connected() && b->writable(); }));
    const float line = b->maxDelayMs();
    check("DL3: the demo's line is the RP2350's 42 ms", line == 42.0f);
    const float before = b->outputDelayMS(0);
    auto *main = qobject_cast<QQuickWindow *>(root());
    QQuickItem *banner = main ? findItem(main->contentItem(), QStringLiteral("delayRefusalBanner")) : nullptr;
    check("DL3: the banner is registered", banner != nullptr);
    const auto shown = [&] { return banner && banner->property("shown").toBool(); };
    check("DL3: no banner before a refusal", b->delayRefusal().isEmpty() && !shown());

    b->setOutputDelay(0, line + 5);
    printf("smoke: DL3 refusal: %s\n", qPrintable(b->delayRefusal()));
    check("DL3: past the line, the delay is not changed", near(b->outputDelayMS(0), before, 1e-4));
    check("DL3: the refusal is in the core's words", b->delayRefusal() == QLatin1String("out.delay: 47 ms is out of range; this device (RP2350) holds 0 to 42 ms"));
    spin(50);
    check("DL3: the banner shows it", shown());
    b->setOutputDelay(0, 2.0f);
    spin(50);
    check("DL3: an accepted delay clears it", b->delayRefusal().isEmpty() && !shown() && near(b->outputDelayMS(0), 2.0, 1e-4));
    b->sendOutputDelayToDevice(0, 100.0f);
    check("DL3: the live path is refused the same way", b->delayRefusal().contains(QLatin1String("holds 0 to 42 ms")) && near(b->outputDelayMS(0), 2.0, 1e-4));
    b->dismissDelayRefusal();
    spin(50);
    check("DL3: Dismiss clears it", b->delayRefusal().isEmpty() && !shown());

    check("DL3: a pasted delay is held to the line",
          DSPiBridge::pastedDelay(85, 42) == 42 && DSPiBridge::pastedDelay(85, 21) == 21 && DSPiBridge::pastedDelay(10, 21) == 10
              && DSPiBridge::pastedDelay(-1, 21) == 0 && DSPiBridge::pastedDelay(std::nanf(""), 21) == 0 && DSPiBridge::pastedDelay(85, 0) == 85);
    const int out0 = 2;   // the app channel of output 0 (DSPiBridge::outputOf)
    b->setOutputDelay(0, 40.0f);
    b->copyChannel(out0);
    b->setOutputDelay(0, 1.0f);
    b->pasteChannel(out0);
    check("DL3: a paste within the line lands whole", near(b->outputDelayMS(0), 40.0, 1e-4) && b->delayRefusal().isEmpty());

    QFile doc(m_out + QStringLiteral("/dl3.dspipreset"));
    const bool written = doc.open(QIODevice::WriteOnly | QIODevice::Truncate)
        && doc.write(R"({"schemaVersion": 1, "channels": [{"isOutput": true, "outputIndex": 0, "name": "Sub", "outputDelayMs": 85}]})") > 0;
    doc.close();
    check("DL3: the document is written", written);
    const QVariantMap r = m_.config->importConfiguration(QUrl::fromLocalFile(doc.fileName()), false, false);
    const QStringList lines = r.value("lines").toStringList();
    printf("smoke: DL3 import: %s\n", qPrintable(lines.join(QLatin1String(" | "))));
    check("DL3: the import shortens the delay to the line", r.value("ok").toBool() && near(b->outputDelayMS(0), 42.0, 1e-4));
    check("DL3: the report names it",
          lines.contains(QStringLiteral("Delays shortened to the 42 ms this device (RP2350) holds: Sub output delay (85 ms).")));

    b->setOutputDelay(0, before);
}
