// -- WP QP8 -- The subharmonic synthesizer, the tube modeller, the upmixer and
// the output limiter's settings on the demo device: the maths against the
// Mac's formulas, every window's controls (each write checked in the device's
// state), solo cleared on close, the polls, the limiter's batch actions with
// "Protect all outputs", the save bar's Revert putting the limiters back, and
// read-only firmware.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "HardwareController.h"
#include "Shell.h"
#include "ToolsController.h"

#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <cmath>
#include <functional>

namespace {

using Tools = ToolsController;

bool qp8Near(double a, double b, double tolerance = 0.01) { return std::fabs(a - b) <= tolerance; }

// A window of this package, and how a person reaches its parts
struct Qp8Window {
    SmokeDriver &d;
    QQuickWindow *w = nullptr;

    QObject *obj(const QString &name) const {
        if (!w) return nullptr;
        if (QObject *o = w->findChild<QObject *>(name)) return o;
        return d.item(name, w->contentItem());
    }
    // -- WP LAY1a -- what the window's own properties were on (the window until it has a panel)
    QObject *panel() const {
        QObject *p = w ? w->property("content").value<QQuickItem *>() : nullptr;
        return p ? p : w;
    }
    QQuickItem *item(const QString &name) const { return qobject_cast<QQuickItem *>(obj(name)); }
    bool shows(const QString &name) const {
        QQuickItem *it = item(name);
        return it && it->isVisible();
    }
    // Scrolls the nearest Flickable so `it` is in view
    void reveal(QQuickItem *it) const {
        for (QQuickItem *flick = it->parentItem(); flick; flick = flick->parentItem()) {
            if (!flick->inherits("QQuickFlickable")) continue;
            auto *content = flick->property("contentItem").value<QQuickItem *>();
            if (!content) return;
            const qreal top = it->mapToItem(content, QPointF(0, 0)).y();
            const qreal room = flick->height();
            qreal y = flick->property("contentY").toReal();
            if (top < y) y = top;
            else if (top + it->height() > y + room) y = top + it->height() - room;
            const qreal most = qMax<qreal>(0, flick->property("contentHeight").toReal() - room);
            flick->setProperty("contentY", qBound<qreal>(0, y, most));
            d.spin(30);
            return;
        }
    }
    // A real click in the middle of the item
    void click(const QString &name) const {
        QQuickItem *it = item(name);
        d.check(QStringLiteral("qp8: %1 is there to click").arg(name), it && it->isVisible());
        if (!it || !w) return;
        reveal(it);
        QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, it->mapToScene(QPointF(it->width() / 2, it->height() / 2)).toPoint());
        d.spin(60);
        d.waitIdle();
    }
    void trigger(const QString &name) const {
        QObject *o = obj(name);
        d.check(QStringLiteral("qp8: the menu row %1 exists").arg(name), o != nullptr);
        if (o) QMetaObject::invokeMethod(o, "triggered");
        d.spin(30);
        d.waitIdle();
    }
    void commit(const QString &row, double v) const {
        QObject *o = obj(row);
        d.check(QStringLiteral("qp8: the row %1 exists").arg(row), o != nullptr);
        if (o) QMetaObject::invokeMethod(o, "commit", Q_ARG(double, v));
        d.waitIdle();
    }
    void live(const QString &row, double v) const {
        if (QObject *o = obj(row)) QMetaObject::invokeMethod(o, "live", Q_ARG(double, v));
        d.waitIdle();
    }
    QString text(const QString &name) const {
        QObject *o = obj(name);
        return o ? o->property("text").toString() : QString();
    }
};

const DspState *qp8State(SmokeDriver &d) { return d.app().device->state(); }

bool qp8Wait(SmokeDriver &d, const std::function<bool(const DspState &)> &done) {
    return d.waitFor(3000, [&]() {
        d.waitIdle(1000);
        const DspState *s = qp8State(d);
        return s && done(*s);
    });
}

// A screenshot in the dark scheme and one in the light (<file>-light.png)
bool qp8Grab(SmokeDriver &d, QQuickWindow *w, const QString &file) {
    bool ok = d.grab(w, file + QStringLiteral(".png"));
    const Qt::ColorScheme before = d.app().shell->colorSchemeOverride();   // -- WP QPI3 --
    d.app().shell->setColorSchemeOverride(Qt::ColorScheme::Light);
    d.spin(150);
    ok = d.grab(w, file + QStringLiteral("-light.png")) && ok;
    d.app().shell->setColorSchemeOverride(before);
    d.spin(150);
    return ok;
}

Qp8Window qp8Open(SmokeDriver &d, const char *action, const char *type) {
    Qp8Window win{ d };
    win.w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QString::fromLatin1(action)));
    d.check(QStringLiteral("qp8: %1 opens the %2").arg(QLatin1String(action), QLatin1String(type)),
            win.w && d.waitFor(3000, [&]() { return win.w->isVisible(); }) && win.w->objectName() == QLatin1String(type));
    if (win.w) d.spin(150);
    return win;
}

float sub(const DspState &s, int id) { return s.subharm[id]; }
float tube(const DspState &s, int id) { return s.tube[id]; }
float up(const DspState &s, int id) { return s.upmix[id]; }

// ---- The maths, against the Mac's formulas ----

void qp8Maths(SmokeDriver &d) {
    d.check(QStringLiteral("qp8: the LF bell peaks at the boost at 70 Hz"), qp8Near(Tools::subharmBellDb(3, 70), 3, 1e-6));
    d.check(QStringLiteral("qp8: the LF bell is gentle an octave away and off at 0 dB"),
            Tools::subharmBellDb(3, 280) < 1 && Tools::subharmBellDb(0, 70) == 0);
    d.check(QStringLiteral("qp8: a sub at 0 dB comes out 1.4 dB below its band (the divider's 0.849)"),
            qp8Near(Tools::subharmSubDb(0, 0, 30), -1.42, 0.01));
    d.check(QStringLiteral("qp8: the band graph's axis runs 16 to 250 Hz"),
            Tools::subharmX(16) == 0 && qp8Near(Tools::subharmX(250), 1, 1e-9) && Tools::subharmX(70) > 0.5);

    // The tube shaper: unity small-signal gain at the default 12AX7, symmetric without bias or asymmetry
    const double g = Tools::tubeOutput(-12, 10, 3, 40, 100, 0, 0.001) / 0.001;
    d.check(QStringLiteral("qp8: the tube keeps clean material at its level"), qp8Near(g, 1, 0.05));
    double second = 0, third = 0;
    Tools::tubeHarmonics(6, 0, 0, 50, 100, 0, second, third);
    d.check(QStringLiteral("qp8: a symmetric stage has no second harmonic, only a third"), second <= -100 && third > -100);
    Tools::tubeHarmonics(6, 30, 6, 40, 100, 0, second, third);
    d.check(QStringLiteral("qp8: bias and asymmetry give the even harmonic"), second > -100);
    Tools::tubeHarmonics(-12, 10, 3, 40, 0, 0, second, third);
    d.check(QStringLiteral("qp8: a dry mix has no harmonics"), second <= -100 && third <= -100);
    const QVariantMap lift = Tools::transformerLift(2);
    d.check(QStringLiteral("qp8: damping 2 lifts 2.5 dB at resonance and 1.6 dB at the top"),
            qp8Near(lift.value(QStringLiteral("bell")).toDouble(), 2.50, 0.01) &&
                qp8Near(lift.value(QStringLiteral("top")).toDouble(), 1.58, 0.01));
    // -- WP WEB6 -- the vectors dspi_core::effect_curves' tests pin too (the web's copy of these curves)
    d.check(QStringLiteral("web6: the LF bell and the sub match the core's vectors"),
            qp8Near(Tools::subharmBellDb(3, 40), 1.412553, 1e-5) && qp8Near(Tools::subharmBellDb(6, 100), 4.157328, 1e-5) &&
                qp8Near(Tools::subharmSubDb(0, 0, 30), -1.421846, 1e-5));
    d.check(QStringLiteral("web6: the tube shaper matches the core's vectors"), [&]() {
        double s2 = 0, s3 = 0, d2 = 0, d3 = 0;
        Tools::tubeHarmonics(6, 30, 6, 40, 100, 0, s2, s3);
        Tools::tubeHarmonics(-12, 10, 3, 40, 100, 0, d2, d3);
        return qp8Near(Tools::tubeOutput(-12, 10, 3, 40, 100, 0, 0.5), 0.489851, 1e-5) &&
               qp8Near(Tools::tubeOutput(24, 0, 6, 40, 100, 0, 0.5), 0.038240, 1e-5) &&
               qp8Near(Tools::tubeOutput(24, 0, 6, 40, 100, 0, -0.25), -0.076299, 1e-5) && qp8Near(s2, -13.654910, 1e-4) &&
               qp8Near(s3, -15.928919, 1e-4) && qp8Near(d2, -39.037918, 1e-4) && qp8Near(d3, -43.730812, 1e-4) &&
               qp8Near(lift.value(QStringLiteral("bell")).toDouble(), 2.498775, 1e-5) &&
               qp8Near(lift.value(QStringLiteral("top")).toDouble(), 1.583625, 1e-5);
    }());
    d.check(QStringLiteral("qp8: the tube families follow the Mac's"),
            Tools::tubeFamily(0) == 0 && Tools::tubeFamily(6) == 3 && Tools::tubeFamily(10) == 4 && Tools::tubeFamily(16) == 9);

    Tools *t = Registrar::instance<Tools>();
    const QVariantMap graph = t->tubeGraph(-12, 10, 3, 40, 100, 0);
    const QVariantList curve = graph.value(QStringLiteral("curve")).toList();
    bool inside = curve.size() == 241 * 2;
    for (const QVariant &v : curve) inside = inside && v.toDouble() >= 0 && v.toDouble() <= 1;
    d.check(QStringLiteral("qp8: the transfer curve has 241 points, all on the graph"), inside);
    d.check(QStringLiteral("qp8: hard drive shades both knees"), [&]() {
        const QVariantMap hot = t->tubeGraph(24, 0, 6, 40, 100, 0);
        return !std::isnan(hot.value(QStringLiteral("clipPos")).toDouble()) &&
               !std::isnan(hot.value(QStringLiteral("clipNeg")).toDouble());
    }());
    const QVariantMap sg = t->subharmGraph(0, -6, -30, 3, -12);
    const QVariantList blocks = sg.value(QStringLiteral("blocks")).toList();
    d.check(QStringLiteral("qp8: the band graph draws two subs, the bell and the ceiling"),
            blocks.size() == 3 && !blocks[0].toMap().value(QStringLiteral("off")).toBool() &&
                blocks[2].toMap().value(QStringLiteral("off")).toBool() && sg.value(QStringLiteral("bell")).toList().size() == 97 * 2 &&
                sg.value(QStringLiteral("ceilingOn")).toBool());
    d.check(QStringLiteral("qp8: every band off says so and draws no ceiling"), [&]() {
        const QVariantMap off = t->subharmGraph(-30, -30, -30, 0, -12);
        return off.value(QStringLiteral("allOff")).toBool() && !off.value(QStringLiteral("ceilingOn")).toBool();
    }());
}

// ---- Subharmonic synthesizer ----

void qp8Subharmonic(SmokeDriver &d) {
    Tools *t = Registrar::instance<Tools>();
    const DspState s0 = *qp8State(d);
    d.check(QStringLiteral("qp8: the demo has the subharmonic synthesizer with the V30 controls"),
            t->subharmSupported() && t->subharmExtended());
    Qp8Window win = qp8Open(d, "tools.subharmonic", "SubharmonicWindow");
    if (!win.w) return;
    d.check(QStringLiteral("qp8: the subharmonic window is the Mac's size"),
            win.w->minimumWidth() == 740 && win.w->minimumHeight() == 612 && win.w->title() == QLatin1String("Subharmonic Synthesizer"));
    d.check(QStringLiteral("qp8: the body shows, not the note"),
            win.shows(QStringLiteral("SubharmonicWindow.body")) && !win.shows(QStringLiteral("SubharmonicWindow.unsupported")));

    if (sub(s0, SUBHARM_PARAM_ENABLED) != 0) win.click(QStringLiteral("SubharmonicWindow.header.switch"));
    win.click(QStringLiteral("SubharmonicWindow.header.switch"));
    d.check(QStringLiteral("qp8: the switch turns the synthesizer on"),
            qp8Wait(d, [](const DspState &s) { return sub(s, SUBHARM_PARAM_ENABLED) != 0; }));

    win.commit(QStringLiteral("SubharmonicWindow.low"), -6);
    d.check(QStringLiteral("qp8: the 24-36 Hz level writes"), qp8Wait(d, [](const DspState &s) { return qp8Near(sub(s, SUBHARM_PARAM_LOW), -6); }));
    win.live(QStringLiteral("SubharmonicWindow.high"), 3);
    d.check(QStringLiteral("qp8: a drag's value goes out live"), qp8Wait(d, [](const DspState &s) { return qp8Near(sub(s, SUBHARM_PARAM_HIGH), 3); }));
    win.commit(QStringLiteral("SubharmonicWindow.high"), 50);
    d.check(QStringLiteral("qp8: a typed 50 dB is held to +12"), qp8Wait(d, [](const DspState &s) { return qp8Near(sub(s, SUBHARM_PARAM_HIGH), 12); }));
    win.commit(QStringLiteral("SubharmonicWindow.top"), -30);
    d.check(QStringLiteral("qp8: a band at its floor reads Off"),
            qp8Wait(d, [](const DspState &s) { return sub(s, SUBHARM_PARAM_TOP) <= -30; }) &&
                d.waitFor(1000, [&]() { return win.shows(QStringLiteral("SubharmonicWindow.top.off")); }) &&
                !win.shows(QStringLiteral("SubharmonicWindow.low.off")));
    // The demo does not answer REQ_GET_SUBHARM_HEADROOM (fake.rs), so the read
    // fails and the readout keeps "none"; on a device it shows "+x.x dB"
    d.spin(300);
    d.check(QStringLiteral("qp8: the headroom readout shows the device's figure, or none"),
            t->subharmHeadroom() > 0 ? win.text(QStringLiteral("SubharmonicWindow.headroom.value")).startsWith(QLatin1Char('+'))
                                     : win.text(QStringLiteral("SubharmonicWindow.headroom.value")) == QLatin1String("none"));

    win.trigger(QStringLiteral("SubharmonicWindow.startingPoint.3"));
    d.check(QStringLiteral("qp8: Cinema LFE sets +3 / -12 / 0 and the top band off"), qp8Wait(d, [](const DspState &s) {
        return qp8Near(sub(s, SUBHARM_PARAM_LOW), 3) && qp8Near(sub(s, SUBHARM_PARAM_HIGH), -12) &&
               qp8Near(sub(s, SUBHARM_PARAM_BOOST), 0) && sub(s, SUBHARM_PARAM_TOP) <= -30;
    }));
    win.commit(QStringLiteral("SubharmonicWindow.boost"), 4.5);
    d.check(QStringLiteral("qp8: the 70 Hz bell writes"), qp8Wait(d, [](const DspState &s) { return qp8Near(sub(s, SUBHARM_PARAM_BOOST), 4.5); }));

    // Selectivity: depth and hold only while a mode favours something
    d.check(QStringLiteral("qp8: all material hides depth and hold"),
            sub(*qp8State(d), SUBHARM_PARAM_SELECT) != 0 || !win.shows(QStringLiteral("SubharmonicWindow.depth")));
    win.click(QStringLiteral("SubharmonicWindow.mode.1"));
    d.check(QStringLiteral("qp8: Percussive writes and shows depth and hold"),
            qp8Wait(d, [](const DspState &s) { return qp8Near(sub(s, SUBHARM_PARAM_SELECT), 1); }) &&
                win.shows(QStringLiteral("SubharmonicWindow.depth")) && win.shows(QStringLiteral("SubharmonicWindow.hold")) &&
                win.text(QStringLiteral("SubharmonicWindow.mode.summary")).contains(QLatin1String("kicks, not the bass line")));
    win.commit(QStringLiteral("SubharmonicWindow.hold"), 1000);
    d.check(QStringLiteral("qp8: the hold is held to 400 ms"), qp8Wait(d, [](const DspState &s) { return qp8Near(sub(s, SUBHARM_PARAM_HOLD), 400); }));
    win.commit(QStringLiteral("SubharmonicWindow.depth"), 40);
    d.check(QStringLiteral("qp8: the depth writes"), qp8Wait(d, [](const DspState &s) { return qp8Near(sub(s, SUBHARM_PARAM_DEPTH), 40); }));
    win.click(QStringLiteral("SubharmonicWindow.mode.0"));
    d.check(QStringLiteral("qp8: back to all material hides them again"),
            qp8Wait(d, [](const DspState &s) { return sub(s, SUBHARM_PARAM_SELECT) == 0; }) &&
                d.waitFor(1000, [&]() { return !win.shows(QStringLiteral("SubharmonicWindow.depth")); }));
    win.commit(QStringLiteral("SubharmonicWindow.ceiling"), -12);
    d.check(QStringLiteral("qp8: the sub ceiling writes"), qp8Wait(d, [](const DspState &s) { return qp8Near(sub(s, SUBHARM_PARAM_CEILING), -12); }));

    // Outputs
    win.trigger(QStringLiteral("SubharmonicWindow.outputs.sub"));
    d.check(QStringLiteral("qp8: Sub only (recommended) leaves the PDM sub alone in the mask"),
            qp8Wait(d, [&](const DspState &s) { return int(sub(s, SUBHARM_PARAM_MASK)) == (1 << t->pdmOutput()); }));
    win.click(QStringLiteral("SubharmonicWindow.output.0"));
    d.check(QStringLiteral("qp8: an output chip adds its output"),
            qp8Wait(d, [&](const DspState &s) { return int(sub(s, SUBHARM_PARAM_MASK)) == ((1 << t->pdmOutput()) | 1); }));
    QObject *chip = win.obj(QStringLiteral("SubharmonicWindow.output.0"));
    d.check(QStringLiteral("qp8: a chip's tooltip names its output"), chip && chip->property("toolTip").toString() == t->outputName(0));
    d.check(QStringLiteral("qp8: the sub meter polls while the window shows"),
            d.waitFor(3000, [&]() { return t->subharmMeter().size() == t->numOutputs(); }));
    const bool link = sub(*qp8State(d), SUBHARM_PARAM_LINK) != 0;
    win.click(QStringLiteral("SubharmonicWindow.linkPairs"));
    d.check(QStringLiteral("qp8: the pair link switches"),
            qp8Wait(d, [&](const DspState &s) { return (sub(s, SUBHARM_PARAM_LINK) != 0) != link; }));

    // Solo latches, and closing the window switches it off
    win.click(QStringLiteral("SubharmonicWindow.solo"));
    d.check(QStringLiteral("qp8: Solo takes the program off the outputs"), qp8Wait(d, [](const DspState &s) { return s.subharm_solo; }));
    d.check(QStringLiteral("qp8: screenshot of the subharmonic synthesizer"), qp8Grab(d, win.w, QStringLiteral("window-subharmonic")));
    win.w->close();
    d.check(QStringLiteral("qp8: closing the window switches Solo off"), qp8Wait(d, [](const DspState &s) { return !s.subharm_solo; }));
    d.check(QStringLiteral("qp8: and the sub meter stops"), d.waitFor(1000, [&]() { return t->subharmMeter().isEmpty(); }));

    for (const char *key : { "low", "high", "top", "boost", "select", "depth", "hold", "ceiling", "link", "mask", "enabled" }) {
        static const QHash<QString, int> ids{ { "enabled", SUBHARM_PARAM_ENABLED }, { "mask", SUBHARM_PARAM_MASK },
                                              { "low", SUBHARM_PARAM_LOW },         { "high", SUBHARM_PARAM_HIGH },
                                              { "top", SUBHARM_PARAM_TOP },         { "boost", SUBHARM_PARAM_BOOST },
                                              { "select", SUBHARM_PARAM_SELECT },   { "depth", SUBHARM_PARAM_DEPTH },
                                              { "hold", SUBHARM_PARAM_HOLD },       { "ceiling", SUBHARM_PARAM_CEILING },
                                              { "link", SUBHARM_PARAM_LINK } };
        t->setSubharm(QString::fromLatin1(key), s0.subharm[ids.value(QString::fromLatin1(key))]);
    }
    d.check(QStringLiteral("qp8: the subharmonic settings are put back"),
            qp8Wait(d, [&](const DspState &s) { return std::equal(std::begin(s.subharm), std::end(s.subharm), std::begin(s0.subharm)); }));
}

// ---- Tube modeller ----

void qp8Tube(SmokeDriver &d) {
    Tools *t = Registrar::instance<Tools>();
    const DspState s0 = *qp8State(d);
    d.check(QStringLiteral("qp8: the demo has the tube modeller"), t->tubeSupported());
    Qp8Window win = qp8Open(d, "tools.tube", "TubeWindow");
    if (!win.w) return;
    win.panel()->setProperty("advanced", false);
    d.spin(60);
    d.check(QStringLiteral("qp8: the tube window is the Mac's size and opens in Basic"),
            win.w->minimumWidth() == 740 && win.shows(QStringLiteral("TubeWindow.basic")) && !win.shows(QStringLiteral("TubeWindow.advanced")));

    if (tube(s0, TUBE_PARAM_ENABLED) == 0) win.click(QStringLiteral("TubeWindow.header.switch"));
    d.check(QStringLiteral("qp8: the switch turns the tube on and the heater glows"),
            qp8Wait(d, [](const DspState &s) { return tube(s, TUBE_PARAM_ENABLED) != 0; }) &&
                d.waitFor(2000, [&]() { return win.item(QStringLiteral("TubeWindow.heater"))->opacity() > 0.99; }));
    win.click(QStringLiteral("TubeWindow.type.16"));
    d.check(QStringLiteral("qp8: a shelf chip picks the tube and loads its character"), qp8Wait(d, [](const DspState &s) {
        return tube(s, TUBE_PARAM_TYPE) == 16 && qp8Near(tube(s, TUBE_PARAM_BIAS), 12) && qp8Near(tube(s, TUBE_PARAM_ASYM), 6) &&
               qp8Near(tube(s, TUBE_PARAM_HARDNESS), 10) && qp8Near(tube(s, TUBE_PARAM_SAG), 12);
    }));
    d.check(QStringLiteral("qp8: the showcase names it and draws the directly heated tube"),
            win.text(QStringLiteral("TubeWindow.showcase.name")) == QLatin1String("300B / 2A3") &&
                win.item(QStringLiteral("TubeWindow.tube"))->property("family").toInt() == 9);
    win.commit(QStringLiteral("TubeWindow.basicDrive"), 6);
    d.check(QStringLiteral("qp8: Basic's drive writes"), qp8Wait(d, [](const DspState &s) { return qp8Near(tube(s, TUBE_PARAM_DRIVE), 6); }));
    win.commit(QStringLiteral("TubeWindow.basicMix"), 70);
    d.check(QStringLiteral("qp8: Basic's mix writes"), qp8Wait(d, [](const DspState &s) { return qp8Near(tube(s, TUBE_PARAM_MIX), 70); }));
    d.check(QStringLiteral("qp8: screenshot of the tube modeller, Basic"), qp8Grab(d, win.w, QStringLiteral("window-tube-basic")));

    win.click(QStringLiteral("TubeWindow.mode.1"));
    d.check(QStringLiteral("qp8: Advanced shows every parameter"),
            win.shows(QStringLiteral("TubeWindow.advanced")) && !win.shows(QStringLiteral("TubeWindow.basic")) &&
                win.panel()->property("advanced").toBool());
    d.check(QStringLiteral("qp8: the character says where it comes from"),
            win.text(QStringLiteral("TubeWindow.characterFrom")) == QLatin1String("from 300B / 2A3"));
    win.commit(QStringLiteral("TubeWindow.bias"), 40);
    d.check(QStringLiteral("qp8: a character edit makes the tube Custom"),
            qp8Wait(d, [](const DspState &s) { return qp8Near(tube(s, TUBE_PARAM_BIAS), 40) && tube(s, TUBE_PARAM_TYPE) == 0; }) &&
                win.text(QStringLiteral("TubeWindow.characterFrom")) == QLatin1String("custom"));
    d.check(QStringLiteral("qp8: the harmonics readout shows a second harmonic"),
            win.text(QStringLiteral("TubeWindow.harmonics.second.value")).endsWith(QLatin1String("dB")));
    win.click(QStringLiteral("TubeWindow.rectifier.0"));
    d.check(QStringLiteral("qp8: a solid-state rectifier means no sag"),
            qp8Wait(d, [](const DspState &s) { return tube(s, TUBE_PARAM_RECTIFIER) == 0; }) &&
                win.text(QStringLiteral("TubeWindow.rectifier.summary")).startsWith(QLatin1String("No sag")) &&
                win.item(QStringLiteral("TubeWindow.sag"))->opacity() < 0.6);
    win.click(QStringLiteral("TubeWindow.rectifier.3"));
    d.check(QStringLiteral("qp8: 5Y3 sags deepest and slowest"),
            qp8Wait(d, [](const DspState &s) { return tube(s, TUBE_PARAM_RECTIFIER) == 3; }) &&
                win.text(QStringLiteral("TubeWindow.rectifier.summary")).contains(QLatin1String("x1.3")) &&
                win.text(QStringLiteral("TubeWindow.rectifier.summary")).contains(QLatin1String("300 ms")));
    win.commit(QStringLiteral("TubeWindow.trim"), -40);
    d.check(QStringLiteral("qp8: the trim is held to -12 dB"), qp8Wait(d, [](const DspState &s) { return qp8Near(tube(s, TUBE_PARAM_TRIM), -12); }));

    if (tube(*qp8State(d), TUBE_PARAM_XFMR) == 0) win.click(QStringLiteral("TubeWindow.xfmr"));
    win.commit(QStringLiteral("TubeWindow.damping"), 10);
    d.check(QStringLiteral("qp8: the damping writes and its lift follows"),
            qp8Wait(d, [](const DspState &s) { return qp8Near(tube(s, TUBE_PARAM_DAMPING), 10); }) &&
                win.text(QStringLiteral("TubeWindow.lift")) == QLatin1String("+0.6 dB at resonance, +0.4 dB at the top."));
    win.click(QStringLiteral("TubeWindow.xfmr"));
    d.check(QStringLiteral("qp8: the output stage off hides its controls"),
            qp8Wait(d, [](const DspState &s) { return tube(s, TUBE_PARAM_XFMR) == 0; }) &&
                !win.shows(QStringLiteral("TubeWindow.damping")) && !win.shows(QStringLiteral("TubeWindow.resonance")));

    win.trigger(QStringLiteral("TubeWindow.startingPoint.3"));
    d.check(QStringLiteral("qp8: Guitar-amp style sets every control but the character"), qp8Wait(d, [](const DspState &s) {
        return tube(s, TUBE_PARAM_TYPE) == 1 && qp8Near(tube(s, TUBE_PARAM_DRIVE), 15) && tube(s, TUBE_PARAM_RECTIFIER) == 2 &&
               tube(s, TUBE_PARAM_XFMR) != 0 && qp8Near(tube(s, TUBE_PARAM_DAMPING), 2) &&
               qp8Near(tube(s, TUBE_PARAM_RESONANCE), 100) && qp8Near(tube(s, TUBE_PARAM_MIX), 100) &&
               qp8Near(tube(s, TUBE_PARAM_TRIM), 0) && qp8Near(tube(s, TUBE_PARAM_BIAS), 10);
    }));
    win.trigger(QStringLiteral("TubeWindow.output.noSub"));
    d.check(QStringLiteral("qp8: Exclude sub leaves the PDM output out"),
            qp8Wait(d, [&](const DspState &s) { return int(tube(s, TUBE_PARAM_MASK)) == t->withoutSubMask(); }));
    win.click(QStringLiteral("TubeWindow.output.1"));
    d.check(QStringLiteral("qp8: a tube chip toggles its output"),
            qp8Wait(d, [&](const DspState &s) { return int(tube(s, TUBE_PARAM_MASK)) == (t->withoutSubMask() & ~2); }));
    d.check(QStringLiteral("qp8: screenshot of the tube modeller, Advanced"), qp8Grab(d, win.w, QStringLiteral("window-tube-advanced")));

    win.click(QStringLiteral("TubeWindow.mode.0"));
    win.w->close();
    static const char *const kTubeKeys[TUBE_PARAM_COUNT] = { "enabled", "mask",  "type", "drive",     "bias",
                                                            "asym",    "hardness", "sag", "rectifier", "xfmr",
                                                            "damping", "resonance", "mix", "trim" };
    // The type first: it loads its character, which then comes back as it was
    t->setTube(QStringLiteral("type"), s0.tube[TUBE_PARAM_TYPE]);
    for (int id = 0; id < TUBE_PARAM_COUNT; ++id)
        if (id != TUBE_PARAM_TYPE && !(s0.tube[TUBE_PARAM_TYPE] != 0 && id >= TUBE_PARAM_BIAS && id <= TUBE_PARAM_SAG))
            t->setTube(QString::fromLatin1(kTubeKeys[id]), s0.tube[id]);
    d.check(QStringLiteral("qp8: the tube settings are put back"),
            qp8Wait(d, [&](const DspState &s) { return std::equal(std::begin(s.tube), std::end(s.tube), std::begin(s0.tube)); }));
}

// ---- Stereo upmixer ----

void qp8Upmixer(SmokeDriver &d) {
    Tools *t = Registrar::instance<Tools>();
    const DspState s0 = *qp8State(d);
    d.check(QStringLiteral("qp8: the demo (RP2350) has the upmixer"), t->upmixSupported());
    Qp8Window win = qp8Open(d, "tools.upmixer", "UpmixerWindow");
    if (!win.w) return;
    d.check(QStringLiteral("qp8: the upmixer window is 400 wide"),
            win.w->width() == 400 && win.w->minimumWidth() == 400 && win.w->maximumWidth() == 400);
    if (up(s0, UPMIX_PARAM_ENABLED) == 0) win.click(QStringLiteral("UpmixerWindow.header.switch"));
    d.check(QStringLiteral("qp8: the switch turns the upmixer on"),
            qp8Wait(d, [](const DspState &s) { return up(s, UPMIX_PARAM_ENABLED) != 0; }));
    d.check(QStringLiteral("qp8: the status polls while the window shows"),
            d.waitFor(3000, [&]() { return t->upmixStatus().contains(QStringLiteral("parked")); }) &&
                !win.text(QStringLiteral("UpmixerWindow.status.text")).isEmpty());

    win.click(QStringLiteral("UpmixerWindow.centre.2"));
    d.check(QStringLiteral("qp8: Logician centre shows the steering controls"),
            qp8Wait(d, [](const DspState &s) { return up(s, UPMIX_PARAM_CENTER_MODE) == 1; }) &&
                win.shows(QStringLiteral("UpmixerWindow.threshold")) && win.shows(QStringLiteral("UpmixerWindow.detector")));
    win.click(QStringLiteral("UpmixerWindow.centre.1"));
    d.check(QStringLiteral("qp8: Sinner centre hides them"),
            qp8Wait(d, [](const DspState &s) { return up(s, UPMIX_PARAM_CENTER_MODE) == 0; }) &&
                !win.shows(QStringLiteral("UpmixerWindow.threshold")) && win.shows(QStringLiteral("UpmixerWindow.strength")));
    win.click(QStringLiteral("UpmixerWindow.centre.0"));
    d.check(QStringLiteral("qp8: centre Off (wire 2) drops the whole centre section"),
            qp8Wait(d, [](const DspState &s) { return up(s, UPMIX_PARAM_CENTER_MODE) == 2; }) &&
                !win.shows(QStringLiteral("UpmixerWindow.centreSection")));
    win.click(QStringLiteral("UpmixerWindow.centre.2"));
    win.commit(QStringLiteral("UpmixerWindow.strength"), 60);
    d.check(QStringLiteral("qp8: the strength writes"), qp8Wait(d, [](const DspState &s) { return qp8Near(up(s, UPMIX_PARAM_STRENGTH), 60); }));
    win.commit(QStringLiteral("UpmixerWindow.presence"), 2.3);
    d.check(QStringLiteral("qp8: the presence goes in 0.5 dB steps"),
            qp8Wait(d, [](const DspState &s) { return qp8Near(up(s, UPMIX_PARAM_PRESENCE), 2.5); }));
    win.commit(QStringLiteral("UpmixerWindow.threshold"), 99);
    d.check(QStringLiteral("qp8: the correlation threshold is held to 95 %"),
            qp8Wait(d, [](const DspState &s) { return qp8Near(up(s, UPMIX_PARAM_THRESHOLD), 95); }));
    win.click(QStringLiteral("UpmixerWindow.surround.2"));
    win.commit(QStringLiteral("UpmixerWindow.lpf"), 9000);
    d.check(QStringLiteral("qp8: the surround's low-pass writes"),
            qp8Wait(d, [](const DspState &s) { return qp8Near(up(s, UPMIX_PARAM_SURROUND_LPF), 9000) && up(s, UPMIX_PARAM_SURROUND_MODE) == 2; }));
    win.click(QStringLiteral("UpmixerWindow.surround.0"));
    d.check(QStringLiteral("qp8: surround Off drops the surround section"),
            qp8Wait(d, [](const DspState &s) { return up(s, UPMIX_PARAM_SURROUND_MODE) == 0; }) &&
                !win.shows(QStringLiteral("UpmixerWindow.surroundSection")));
    win.click(QStringLiteral("UpmixerWindow.surround.2"));
    d.check(QStringLiteral("qp8: screenshot of the upmixer"), qp8Grab(d, win.w, QStringLiteral("window-upmixer")));
    win.w->close();

    static const char *const kUpmixKeys[UPMIX_PARAM_COUNT] = { "enabled", "center",  "surround", "strength", "width",
                                                              "threshold", "attack", "release",  "detector", "delay",
                                                              "hpf",     "lpf",     "decorrelation", "presence" };
    for (int id = 0; id < UPMIX_PARAM_COUNT; ++id) t->setUpmix(QString::fromLatin1(kUpmixKeys[id]), s0.upmix[id]);
    d.check(QStringLiteral("qp8: the upmixer settings are put back"),
            qp8Wait(d, [&](const DspState &s) { return std::equal(std::begin(s.upmix), std::end(s.upmix), std::begin(s0.upmix)); }));
}

// ---- Output limiters ----

bool qp8Limiters(const DspState &s, int n, const std::function<bool(int)> &each) {
    for (int o = 0; o < n; ++o)
        if (!each(o)) return false;
    return true;
}

void qp8Limiter(SmokeDriver &d) {
    Tools *t = Registrar::instance<Tools>();
    const int n = t->numOutputs();
    const DspState lim0 = *qp8State(d);
    d.check(QStringLiteral("qp8: the demo has the output limiters"), t->limiterSupported() && n >= 5);
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("smoke.channel")));
    d.check(QStringLiteral("qp8: the output page opens"), w && d.waitFor(3000, [&]() { return w->isVisible(); }));
    if (!w) return;
    w->setProperty("page", QStringLiteral("output"));
    w->setProperty("output", 0);
    d.spin(150);
    Qp8Window page{ d, w };
    QObject *popover = page.obj(QStringLiteral("LimiterSettings"));
    QObject *cell = page.obj(QStringLiteral("OutputCard.limiter"));
    d.check(QStringLiteral("qp8: the output card carries the limiter's popover"), popover && cell);
    if (!popover || !cell) return;

    QMetaObject::invokeMethod(cell, "secondaryClicked");
    d.check(QStringLiteral("qp8: a right-click on the limiter cell opens its settings"),
            d.waitFor(2000, [&]() { return popover->property("opened").toBool(); }) &&
                page.text(QStringLiteral("LimiterSettings.output")) == t->outputName(0));
    d.check(QStringLiteral("qp8: the popover is 320 wide"), qp8Near(popover->property("width").toDouble(), 320));

    const bool on0 = qp8State(d)->limiter_enabled[0];
    if (on0) {
        page.click(QStringLiteral("LimiterSettings.enabled"));
        qp8Wait(d, [](const DspState &s) { return !s.limiter_enabled[0]; });
    }
    QQuickItem *settings = page.item(QStringLiteral("LimiterSettings.settings"));
    d.check(QStringLiteral("qp8: off, the settings are greyed out and disabled"),
            settings && d.waitFor(1000, [&]() { return !settings->isEnabled() && settings->opacity() < 0.45; }));
    page.click(QStringLiteral("LimiterSettings.enabled"));
    d.check(QStringLiteral("qp8: the popover's switch turns the limiter on"),
            qp8Wait(d, [](const DspState &s) { return s.limiter_enabled[0]; }) &&
                d.waitFor(1000, [&]() { return page.item(QStringLiteral("LimiterSettings.settings"))->isEnabled(); }));
    page.commit(QStringLiteral("LimiterSettings.threshold"), -6);
    d.check(QStringLiteral("qp8: the threshold writes"), qp8Wait(d, [](const DspState &s) { return qp8Near(s.limiter_threshold_db[0], -6); }));
    page.commit(QStringLiteral("LimiterSettings.release"), 5000);
    d.check(QStringLiteral("qp8: the release is held to 1000 ms"),
            qp8Wait(d, [](const DspState &s) { return qp8Near(s.limiter_release_ms[0], 1000); }));

    page.trigger(QStringLiteral("LimiterSettings.unlinkAll"));
    d.check(QStringLiteral("qp8: Unlink all outputs clears every group"),
            qp8Wait(d, [&](const DspState &s) { return qp8Limiters(s, n, [&](int o) { return s.limiter_link_group[o] == 0; }); }) &&
                page.text(QStringLiteral("LimiterSettings.groupSummary")) == QLatin1String("Not linked."));
    page.click(QStringLiteral("LimiterSettings.group.3"));
    d.check(QStringLiteral("qp8: a link group alone says so"),
            qp8Wait(d, [](const DspState &s) { return s.limiter_link_group[0] == 3; }) &&
                page.text(QStringLiteral("LimiterSettings.groupSummary")) == QLatin1String("No other outputs in group 3."));

    QMetaObject::invokeMethod(page.obj(QStringLiteral("LimiterSettings.copyToAll")), "click");
    d.waitIdle();
    d.check(QStringLiteral("qp8: Copy to all outputs gives every output this one's values"), qp8Wait(d, [&](const DspState &s) {
        return qp8Limiters(s, n, [&](int o) {
            return s.limiter_enabled[o] && qp8Near(s.limiter_threshold_db[o], -6) && qp8Near(s.limiter_release_ms[o], 1000);
        });
    }));
    d.check(QStringLiteral("qp8: and leaves the link groups alone"),
            qp8State(d)->limiter_link_group[0] == 3 && qp8State(d)->limiter_link_group[1] == 0);

    page.trigger(QStringLiteral("LimiterSettings.allOff"));
    d.check(QStringLiteral("qp8: Switch every limiter off"),
            qp8Wait(d, [&](const DspState &s) { return qp8Limiters(s, n, [&](int o) { return !s.limiter_enabled[o]; }); }));

    // PRO1 S2: every limiter on at -1 dBFS, the stereo pairs linked, the PDM sub unlinked
    page.trigger(QStringLiteral("LimiterSettings.protectAll"));
    const QVariantList pairs = t->stereoPairGroups();
    d.check(QStringLiteral("qp8: Protect all outputs switches every limiter on at -1 dBFS"), qp8Wait(d, [&](const DspState &s) {
        return qp8Limiters(s, n, [&](int o) { return s.limiter_enabled[o] && qp8Near(s.limiter_threshold_db[o], -1); });
    }));
    d.check(QStringLiteral("qp8: and links the stereo pairs, the sub left out"), qp8Wait(d, [&](const DspState &s) {
        return pairs.size() == n && pairs.value(0).toInt() == 1 && pairs.value(1).toInt() == 1 && pairs.value(2).toInt() == 2 &&
               pairs.value(t->pdmOutput()).toInt() == 0 &&
               qp8Limiters(s, n, [&](int o) { return s.limiter_link_group[o] == pairs.value(o).toInt(); });
    }));
    d.check(QStringLiteral("qp8: the summary names the partner"),
            page.text(QStringLiteral("LimiterSettings.groupSummary")) == QStringLiteral("Linked with %1.").arg(t->outputName(1)));
    d.check(QStringLiteral("qp8: screenshot of the limiter's settings"), qp8Grab(d, w, QStringLiteral("window-limiter-settings")));
    page.click(QStringLiteral("LimiterSettings.enabled"));
    d.check(QStringLiteral("qp8: a linked limiter switches its partner too"),
            qp8Wait(d, [](const DspState &s) { return !s.limiter_enabled[0] && !s.limiter_enabled[1] && s.limiter_enabled[2]; }));
    page.trigger(QStringLiteral("LimiterSettings.unlinkAll"));
    qp8Wait(d, [&](const DspState &s) { return qp8Limiters(s, n, [&](int o) { return s.limiter_link_group[o] == 0; }); });
    QMetaObject::invokeMethod(popover, "close");
    d.spin(60);

    // ---- The save bar's Revert puts the limiters back (Independent output mode) ----
    auto *hw = Registrar::instance<HardwareController>();
    const int mode0 = qp8State(d)->output_config_mode;
    if (mode0 != 0) {
        hw->setDraft(QStringLiteral("outputConfigMode"), 0);
        hw->save();
        d.waitFor(3000, [&]() { return !hw->saving(); });
    }
    if (hw->pending()) hw->revert();
    d.waitFor(3000, [&]() { return !hw->pending(); });
    // A baseline with every output its own: distinct thresholds, unlinked, alternately on
    for (int o = 0; o < n; ++o) {
        t->setLimiterGroup(o, 0);
        t->setLimiterThreshold(o, -2.0 - o);
        t->setLimiterRelease(o, 50.0 + 10 * o);
        t->setLimiterEnabled(o, o % 2 == 0);
    }
    d.waitIdle();
    hw->save();
    d.check(QStringLiteral("qp8: the baseline is kept"), d.waitFor(3000, [&]() { return !hw->saving() && !hw->pending(); }));
    const DspState base = *qp8State(d);
    t->protectAllOutputs();
    d.check(QStringLiteral("qp8: a batch action in Independent mode brings up the save bar"),
            qp8Wait(d, [](const DspState &s) { return s.limiter_link_group[0] == 1 && s.limiter_enabled[1]; }) && hw->pending());
    // Output 1 now holds its own baseline value while still linked to output 0,
    // which does not: putting output 0 back must not drag output 1 along
    t->setLimiterThreshold(0, base.limiter_threshold_db[1]);
    d.check(QStringLiteral("qp8: a linked threshold moves both outputs"),
            qp8Wait(d, [&](const DspState &s) { return qp8Near(s.limiter_threshold_db[0], base.limiter_threshold_db[1]); }));
    hw->revert();
    d.check(QStringLiteral("qp8: Revert puts every limiter back as it was"), qp8Wait(d, [&](const DspState &s) {
        return qp8Limiters(s, n, [&](int o) {
            return s.limiter_enabled[o] == base.limiter_enabled[o] && s.limiter_link_group[o] == base.limiter_link_group[o] &&
                   qp8Near(s.limiter_threshold_db[o], base.limiter_threshold_db[o]) &&
                   qp8Near(s.limiter_release_ms[o], base.limiter_release_ms[o]);
        });
    }) && !hw->pending());
    // Linked groups in the baseline too: a threshold moved on one member of each pair
    t->linkLimiterPairs();
    t->setLimiterThreshold(0, -4);
    t->setLimiterThreshold(2, -7);
    d.waitIdle();
    hw->save();
    d.waitFor(3000, [&]() { return !hw->saving() && !hw->pending(); });
    const DspState linked = *qp8State(d);
    t->unlinkLimiters();
    t->setLimiterThreshold(1, -20);
    t->setLimiterEnabled(3, !linked.limiter_enabled[3]);
    d.waitIdle();
    hw->revert();
    d.check(QStringLiteral("qp8: Revert restores linked groups and their shared values"), qp8Wait(d, [&](const DspState &s) {
        return qp8Limiters(s, n, [&](int o) {
            return s.limiter_enabled[o] == linked.limiter_enabled[o] && s.limiter_link_group[o] == linked.limiter_link_group[o] &&
                   qp8Near(s.limiter_threshold_db[o], linked.limiter_threshold_db[o]);
        });
    }) && !hw->pending());

    // Back as the section found them, in the Revert's order: unlinked, each its own, then the groups
    t->unlinkLimiters();
    for (int o = 0; o < n; ++o) {
        t->setLimiterThreshold(o, lim0.limiter_threshold_db[o]);
        t->setLimiterRelease(o, lim0.limiter_release_ms[o]);
        t->setLimiterEnabled(o, lim0.limiter_enabled[o]);
    }
    d.waitIdle();
    for (int o = 0; o < n; ++o)
        if (lim0.limiter_link_group[o] != 0) t->setLimiterGroup(o, lim0.limiter_link_group[o]);
    d.waitIdle();
    if (hw->pending()) hw->save();
    d.waitFor(3000, [&]() { return !hw->saving(); });
    if (mode0 != 0) {
        hw->setDraft(QStringLiteral("outputConfigMode"), mode0);
        hw->save();
        d.waitFor(3000, [&]() { return !hw->saving(); });
    }
    d.check(QStringLiteral("qp8: the limiters are put back and the bar is clear"), qp8Wait(d, [&](const DspState &s) {
        return qp8Limiters(s, n, [&](int o) {
            return s.limiter_enabled[o] == lim0.limiter_enabled[o] && s.limiter_link_group[o] == lim0.limiter_link_group[o] &&
                   qp8Near(s.limiter_threshold_db[o], lim0.limiter_threshold_db[o]);
        });
    }) && !hw->pending());
    w->close();
}

// ---- Read-only firmware (behaviour.md 14): the notes, nothing writes ----

void qp8ReadOnly(SmokeDriver &d) {
    Tools *t = Registrar::instance<Tools>();
    DeviceModel *device = d.app().device;
    d.app().core->worker()->post([](FfiCore *c) { dspi_demo_open_wire(c, 17); });
    device->refresh();
    d.check(QStringLiteral("qp8: the V17 recording opens read-only"), d.waitFor(5000, [&]() { return device->readOnly(); }));
    d.waitIdle();
    d.check(QStringLiteral("qp8: V17 has none of the four"),
            !t->writable() && !t->subharmSupported() && !t->tubeSupported() && !t->upmixSupported() && !t->limiterSupported());
    Qp8Window sw = qp8Open(d, "tools.subharmonic", "SubharmonicWindow");
    Qp8Window tw = qp8Open(d, "tools.tube", "TubeWindow");
    Qp8Window uw = qp8Open(d, "tools.upmixer", "UpmixerWindow");
    if (!sw.w || !tw.w || !uw.w) return;
    d.check(QStringLiteral("qp8: firmware before V29 gets the subharmonic note"),
            sw.shows(QStringLiteral("SubharmonicWindow.unsupported")) && !sw.shows(QStringLiteral("SubharmonicWindow.body")));
    d.check(QStringLiteral("qp8: firmware before V31 gets the tube note"),
            tw.shows(QStringLiteral("TubeWindow.unsupported")) && !tw.shows(QStringLiteral("TubeWindow.basic")) &&
                !tw.shows(QStringLiteral("TubeWindow.advanced")));
    d.check(QStringLiteral("qp8: the upmixer gets its note"),
            uw.shows(QStringLiteral("UpmixerWindow.unsupported")) && !uw.shows(QStringLiteral("UpmixerWindow.body")));
    d.check(QStringLiteral("qp8: the switches are disabled"),
            !sw.item(QStringLiteral("SubharmonicWindow.header.switch"))->isEnabled() &&
                !tw.item(QStringLiteral("TubeWindow.header.switch"))->isEnabled() &&
                !uw.item(QStringLiteral("UpmixerWindow.header.switch"))->isEnabled());
    const int before = device->settersRun();
    t->setSubharm(QStringLiteral("low"), 3);
    t->setTube(QStringLiteral("drive"), 3);
    t->setUpmix(QStringLiteral("strength"), 30);
    t->setLimiterEnabled(0, true);
    t->protectAllOutputs();
    t->switchLimitersOff();
    d.waitIdle();
    d.check(QStringLiteral("qp8: read-only sends nothing"), device->settersRun() == before);
    d.check(QStringLiteral("qp8: screenshot of the subharmonic note"), d.grab(sw.w, QStringLiteral("window-subharmonic-unsupported.png")));
    sw.w->close();
    tw.w->close();
    uw.w->close();

    device->openDemo();
    d.check(QStringLiteral("qp8: the demo reopens writable"),
            d.waitFor(5000, [&]() { return !device->readOnly() && device->connected(); }));
    d.waitIdle();
}

void qp8Section(SmokeDriver &d) {
    d.check(QStringLiteral("qp8: the tools controller is made"), Registrar::instance<Tools>() != nullptr);
    for (const char *id : { "tools.subharmonic", "tools.tube", "tools.upmixer" })
        d.check(QStringLiteral("qp8: the menu row %1 opens a window").arg(QLatin1String(id)),
                d.app().shell->handles(QString::fromLatin1(id)));
    qp8Maths(d);
    qp8Subharmonic(d);
    qp8Tube(d);
    qp8Upmixer(d);
    qp8Limiter(d);
    qp8ReadOnly(d);
    d.mainWindow()->requestActivate();
    d.spin(100);
}

const SmokeDriver::Section qp8Tools("qp8-tools", 72, &qp8Section);

} // namespace
