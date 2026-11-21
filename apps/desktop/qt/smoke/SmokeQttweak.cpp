// -- WP QTTWEAK -- Classic-layout tweaks from a user's feedback, on the demo
// device: the band list's, the channel strip's and the band bar's fields
// select all their text when Tab or a click brings the focus in; the graph
// has no "Filter Response" heading above it; the output page's header and
// control strip take less height; a mouse drag on a dot moves its band's
// frequency and gain, as a person does it (the pointer rests on the dot
// first); the wheel over a dot changes its Q; the main window fits a 768 px
// screen. Screenshots of the main window at 1920x1080 and 1366x768 (less a
// title bar and a taskbar) go to <out>/qttweak/.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "DeviceModel.h"
#include "GraphController.h"
#include "Registrar.h"
#include "SidebarController.h"
#include "render/PeqOverlay.h"

#include <QDir>
#include <QGuiApplication>
#include <QQuickItem>
#include <QQuickWindow>
#include <QScreen>
#include <QTest>
#include <QWheelEvent>
#include <cmath>

namespace {

QPoint centreOf(QQuickItem *i) { return i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint(); }

QString selectedText(QQuickItem *field) { return field ? field->property("selectedText").toString() : QString(); }
QString textOf(QQuickItem *field) { return field ? field->property("text").toString() : QString(); }
bool allSelected(QQuickItem *field) { return field && !textOf(field).isEmpty() && selectedText(field) == textOf(field); }

// The main window at the size it has on a maximised `screen`-sized desktop:
// less a title bar (32 px) and a taskbar (40 px)
void fit(SmokeDriver &d, QQuickWindow *w, QSize screen) {
    const QSize size(screen.width(), screen.height() - 72);
    w->setMinimumSize(QSize(qMin(w->minimumWidth(), size.width()), qMin(w->minimumHeight(), size.height())));
    w->resize(size);
    d.spin(150);
    d.waitIdle();
    d.spin(150);
}

// The shown item named `name` (a band row names the fields of both its layouts alike)
QQuickItem *shownItem(QQuickItem *from, const QString &name) {
    if (!from) return nullptr;
    if (from->objectName() == name && from->isVisible()) {
        bool shown = true;
        for (QQuickItem *i = from; i && shown; i = i->parentItem()) shown = i->isVisible();
        if (shown) return from;
    }
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children)
        if (QQuickItem *found = shownItem(child, name)) return found;
    return nullptr;
}

// A click into a field without the focus, a second click, then `tabs` presses
// of Tab to the field `nextName`
void selectAllChecks(SmokeDriver &d, QQuickWindow *w, const QString &what, const QString &fieldName, const QString &nextName, int tabs = 1) {
    QQuickItem *field = shownItem(w->contentItem(), fieldName);
    d.check(QStringLiteral("qttweak: %1 is on screen").arg(what), field && field->isVisible() && field->width() > 0);
    if (!field) return;
    // Away from it first, so the click is the one that brings the focus in
    if (auto *ov = d.item(QStringLiteral("PeqEditor.overlay"))) ov->forceActiveFocus();
    d.spin(20);
    QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, centreOf(field));
    d.spin(30);
    d.check(QStringLiteral("qttweak: a click into %1 selects all its text").arg(what), field->hasActiveFocus() && allSelected(field));
    // A second click places the cursor (no selection), as in any text field
    QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, centreOf(field) + QPoint(2, 0));
    d.spin(30);
    d.check(QStringLiteral("qttweak: a second click into %1 places the cursor").arg(what), field->hasActiveFocus() && selectedText(field).isEmpty());
    if (nextName.isEmpty()) {
        QTest::keyClick(w, Qt::Key_Escape);
        return;
    }
    for (int i = 0; i < tabs; ++i) {
        QTest::keyClick(w, Qt::Key_Tab);
        d.spin(30);
    }
    QQuickItem *next = shownItem(w->contentItem(), nextName);
    d.check(QStringLiteral("qttweak: Tab from %1 selects all of the next field's text").arg(what),
            next && next->hasActiveFocus() && allSelected(next));
    QTest::keyClick(w, Qt::Key_Escape);
    d.spin(20);
}

void qttweakSection(SmokeDriver &d) {
    QQuickWindow *w = d.mainWindow();
    auto *sc = Registrar::instance<SidebarController>();
    auto *g = Registrar::instance<GraphController>();
    auto *cc = Registrar::instance<ChannelController>();
    d.check(QStringLiteral("qttweak: the main window and the controllers"), w && sc && g && cc);
    if (!w || !sc || !g || !cc) return;
    d.check(QStringLiteral("qttweak: on the demo"), d.waitFor(10000, [&] { return d.app().device->connected(); }) && d.waitIdle());

    // ---- The window fits a 1366x768 screen with its title bar and taskbar ----
    d.check(QStringLiteral("qttweak: the main window's minimum height fits a 768 px screen (%1)").arg(w->minimumHeight()),
            w->minimumHeight() <= 690);

    const QSize original = w->size();
    const QSize originalMin = w->minimumSize();
    const int out = g->firstOutput();
    sc->select(out);
    d.waitFor(3000, [&] { return d.item(QStringLiteral("OutputDetail")) != nullptr; });
    d.waitIdle();

    // ---- No heading above the graph; the graph keeps its accessible name ----
    QQuickItem *title = d.item(QStringLiteral("GraphHost.title"));
    QQuickItem *plot = d.item(QStringLiteral("GraphHost.plot"));
    QQuickItem *pane = d.item(QStringLiteral("RightPane"));
    d.check(QStringLiteral("qttweak: no \"Filter Response\" heading over the graph"), !title || !title->isVisible());
    d.check(QStringLiteral("qttweak: the graph starts at the top of the right pane"),
            plot && pane && plot->mapToItem(pane, QPointF(0, 0)).y() <= 16);

    // ---- Screenshots, and the output page's rows at both sizes ----
    QDir(d.outDir()).mkpath(QStringLiteral("qttweak"));
    const QList<QSize> screens{ QSize(1920, 1080), QSize(1366, 768) };
    for (const QSize &screen : screens) {
        const QString tag = QStringLiteral("%1x%2").arg(screen.width()).arg(screen.height());
        fit(d, w, screen);
        sc->select(out);
        d.spin(200);
        d.waitIdle();
        d.check(QStringLiteral("qttweak: %1, output page").arg(tag), d.grab(w, QStringLiteral("qttweak/%1-output.png").arg(tag)));
        QQuickItem *card = d.item(QStringLiteral("OutputCard"));
        QQuickItem *enable = d.item(QStringLiteral("OutputDetail.enable"));
        QQuickItem *bands = d.item(QStringLiteral("OutputDetail.bands"));
        QQuickItem *detail = d.item(QStringLiteral("OutputDetail"));
        if (card && enable && bands && detail) {
            const QRectF c = card->mapRectToItem(detail, QRectF(0, 0, card->width(), card->height()));
            const QRectF e = enable->mapRectToItem(detail, QRectF(0, 0, enable->width(), enable->height()));
            const QRectF b = bands->mapRectToItem(detail, QRectF(0, 0, bands->width(), bands->height()));
            std::printf("smoke: qttweak: %s card %.0fx%.0f at y %.0f, enable at y %.0f, bands from y %.0f, %.0f high\n",
                        qPrintable(tag), c.width(), c.height(), c.y(), e.y(), b.y(), b.height());
            d.check(QStringLiteral("qttweak: %1, the output strip is one row (%2 px high)").arg(tag).arg(c.height()), c.height() <= 44);
            // Wide, the card sits in the name's row; at 1366 under it (it was 122 px down before)
            const double top = screen.width() >= 1920 ? 64 : 90;
            d.check(QStringLiteral("qttweak: %1, the band list starts within %2 px of the page's top (%3)").arg(tag).arg(top).arg(b.y()),
                    b.y() <= top + 1);
            d.check(QStringLiteral("qttweak: %1, the strip and the switch stay inside the page").arg(tag),
                    c.right() <= detail->width() + 0.5 && e.right() <= detail->width() + 0.5 && c.left() >= -0.5);
            d.check(QStringLiteral("qttweak: %1, the band list has room for rows (%2 px)").arg(tag).arg(b.height()), b.height() >= 120);
        } else {
            d.check(QStringLiteral("qttweak: %1, the output page's parts").arg(tag), false);
        }
        sc->select(-1);
        d.spin(200);
        d.waitIdle();
        d.check(QStringLiteral("qttweak: %1, overview").arg(tag), d.grab(w, QStringLiteral("qttweak/%1-overview.png").arg(tag)));
    }
    // At the minimum size the output page still lays out
    w->resize(w->minimumSize());
    sc->select(out);
    d.spin(200);
    d.waitIdle();
    if (QQuickItem *bands = d.item(QStringLiteral("OutputDetail.bands")))
        d.check(QStringLiteral("qttweak: at the minimum size %1x%2 the band list still shows rows (%3 px)")
                    .arg(w->width()).arg(w->height()).arg(bands->height()),
                bands->height() >= 80);
    d.grab(w, QStringLiteral("qttweak/minimum-output.png"));
    fit(d, w, QSize(1366, 768));
    sc->select(out);
    d.spin(200);
    d.waitIdle();

    // ---- Dragging a dot with the mouse, the wheel ----
    auto *ov = qobject_cast<PeqOverlay *>(d.object(QStringLiteral("PeqEditor.overlay")));
    d.check(QStringLiteral("qttweak: the graph edits the output's bands"), ov && d.waitFor(3000, [&] { return ov->editedChannel() == out; }));
    // The state is read each time: a device update replaces it
    auto band = [&](int b) { const DspState *s = d.app().device->state(); return s ? s->filters[out][b] : FilterParams{}; };
    if (ov) {
        cc->clearBands(out, false);
        cc->setBandType(out, 0, false, FILTER_PEAKING);
        cc->setBandValue(out, 0, false, QStringLiteral("freq"), 1000);
        cc->setBandValue(out, 0, false, QStringLiteral("q"), 1.0);
        cc->setBandValue(out, 0, false, QStringLiteral("gain"), 0);
        d.waitIdle();
        d.spin(60);
        ov->settle();

        // ---- Select all on focus (the band set above gives the list its fields) ----
        selectAllChecks(d, w, QStringLiteral("the band list's frequency"), QStringLiteral("OutputDetail.bands.band.0.freq.field"),
                        QStringLiteral("OutputDetail.bands.band.0.gain.field"));
        selectAllChecks(d, w, QStringLiteral("the output gain"), QStringLiteral("OutputCard.gainValue.field"), QStringLiteral("OutputCard.delayValue.field"),
                        2); // past the gain slider
        selectAllChecks(d, w, QStringLiteral("the input mix gain"), QStringLiteral("OutputCard.routeGain.0.field"), QString());
        d.waitIdle();
        ov->settle();

        const QPoint from = ov->mapToScene(ov->dotPoint(0)).toPoint();
        // As a person does it: the pointer comes in, rests on the dot, presses and moves
        QTest::mouseMove(w, from + QPoint(-30, 20));
        d.spin(20);
        QTest::mouseMove(w, from);
        d.spin(40);
        QTest::mousePress(w, Qt::LeftButton, Qt::NoModifier, from);
        d.spin(20);
        const QPoint to = from + QPoint(60, -40);
        for (int i = 1; i <= 12; ++i) {
            QTest::mouseMove(w, from + (to - from) * i / 12);
            d.spin(16);
        }
        QTest::mouseRelease(w, Qt::LeftButton, Qt::NoModifier, to);
        d.waitIdle();
        d.spin(60);
        ov->settle();
        const FilterParams b = band(0);
        d.check(QStringLiteral("qttweak: a mouse drag on a dot raises its frequency (%1 Hz)").arg(b.freq), b.freq > 1200);
        d.check(QStringLiteral("qttweak: and its gain (%1 dB)").arg(b.gain), b.gain > 1.0);
        // The wheel over the dot: Q
        const double q0 = b.q;
        const QPoint at = ov->mapToScene(ov->dotPoint(0)).toPoint();
        QTest::mouseMove(w, at);
        d.spin(20);
        QWheelEvent wheel(QPointF(at), w->mapToGlobal(QPointF(at)), QPoint(), QPoint(0, 120), Qt::NoButton, Qt::NoModifier, Qt::NoScrollPhase, false);
        QCoreApplication::sendEvent(w, &wheel);
        d.spin(600);
        d.waitIdle();
        d.check(QStringLiteral("qttweak: the wheel over a dot changes its Q (%1 to %2)").arg(q0).arg(band(0).q),
                std::abs(band(0).q - q0) > 0.01);
        cc->clearBands(out, false);
        d.waitIdle();
    }

    w->setMinimumSize(originalMin);
    w->resize(original);
    sc->select(-1);
    d.spin(100);
}

} // namespace

static const SmokeDriver::Section qttweak("qttweak", 36, &qttweakSection);
