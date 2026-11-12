// -- WP QP2 -- The channel pages on the demo device: the input header (preamp,
// pair link and its question, Clear PEQ), the filter lists (type picker gated
// by firmware, fields, bypass, Enable/Bypass All, Clear All, the XO tab and its
// question, the Linkwitz panel), the output card (routing, gain, delay with
// SL1, mute, limiter), the enable questions, and read-only firmware.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "GraphController.h"
#include "Registrar.h"
#include "Shell.h"

#include <QQuickItem>
#include <algorithm>
#include <QQuickWindow>
#include <QTest>
#include <cmath>

namespace {

const Registrar::Window channelWindow("smoke.channel", "SmokeChannelWindow");

QObject *find(QQuickWindow *w, const QString &name, QObject *scope = nullptr) {
    if (scope) return scope->findChild<QObject *>(name);
    if (!w) return nullptr;
    if (QObject *o = w->findChild<QObject *>(name)) return o;
    std::function<QQuickItem *(QQuickItem *)> walk = [&](QQuickItem *i) -> QQuickItem * {
        if (!i) return nullptr;
        if (i->objectName() == name) return i;
        for (QQuickItem *c : i->childItems())
            if (QQuickItem *f = walk(c)) return f;
        return nullptr;
    };
    return walk(w->contentItem());
}

bool near(double a, double b, double tol = 1e-3) { return std::abs(a - b) <= tol; }

struct Pages {
    SmokeDriver &d;
    QQuickWindow *w;
    ChannelController *cc;
    DeviceModel *device;

    const DspState *s() const { return device->state(); }
    QObject *obj(const QString &name, QObject *scope = nullptr) const { return find(w, name, scope); }
    void click(const QString &name, QObject *scope = nullptr) {
        QObject *o = obj(name, scope);
        d.check(QStringLiteral("qp2: %1 exists").arg(name), o != nullptr);
        if (o) QMetaObject::invokeMethod(o, "click");
        d.waitIdle();
    }
    // Types into a ChannelValueField and presses Enter
    void type(const QString &field, const QString &text, QObject *scope = nullptr) {
        QObject *o = obj(field + QStringLiteral(".field"), scope);
        d.check(QStringLiteral("qp2: %1 exists").arg(field), o != nullptr);
        if (!o) return;
        o->setProperty("text", text);
        QMetaObject::invokeMethod(o, "accepted");
        d.waitIdle();
    }
    bool opened(QObject *popup) {
        return popup && d.waitFor(2000, [&]() { return popup->property("opened").toBool(); });
    }
    void answer(QObject *dialog, const char *a) {
        if (dialog) QMetaObject::invokeMethod(dialog, "finish", Q_ARG(QVariant, QString::fromLatin1(a)));
        d.waitFor(2000, [&]() { return dialog && !dialog->property("visible").toBool(); });
        d.waitIdle();
    }
    void show(const char *page, int index = 0) {
        w->setProperty("page", QString::fromLatin1(page));
        w->setProperty(page, index);
        d.spin(150);
    }
};

void filterTypes(Pages &p) {
    SmokeDriver &d = p.d;
    const QVariantList in = p.cc->availableTypes(false), out = p.cc->availableTypes(true);
    d.check(QStringLiteral("qp2: inputs are not offered the Linkwitz Transform"), !in.contains(FILTER_LINKWITZ_TRANSFORM));
    d.check(QStringLiteral("qp2: V32 outputs are offered every PEQ type"), out.size() == 14 && out.contains(FILTER_LINKWITZ_TRANSFORM));
    d.check(QStringLiteral("qp2: no crossover type in the PEQ picker"),
            std::none_of(out.cbegin(), out.cend(), [](const QVariant &t) { return t.toInt() >= FILTER_XOVER_FIRST; }));
    const QVariantList menu = p.cc->typeMenu(false);
    d.check(QStringLiteral("qp2: the input type menu has the Mac's eight shapes"), menu.size() == 8);
    const QVariantMap shelf = menu.value(2).toMap();
    d.check(QStringLiteral("qp2: Low Shelf opens a submenu of its two orders"),
            shelf.value(QStringLiteral("name")).toString() == QLatin1String("Low Shelf") &&
                shelf.value(QStringLiteral("variants")).toList().size() == 2);
    d.check(QStringLiteral("qp2: type names come from the core"), p.cc->typeName(FILTER_LOWPASS) == QLatin1String("High Cut (12dB)"));
    d.check(QStringLiteral("qp2: crossover codes: LR4 high pass is 35, BW1 low pass 40, BES8 high pass 63"),
            ChannelController::crossoverType(1, 4, false) == 35 && ChannelController::crossoverType(2, 1, true) == 40 &&
                ChannelController::crossoverType(3, 8, false) == 63 && ChannelController::crossoverType(1, 3, true) == -1);
    d.check(QStringLiteral("qp2: Linkwitz boost is 40 log10(f0/fp)"), near(ChannelController::linkwitzBoost(40, 20), 12.041, 0.01));

    // The menu a row builds when it opens
    QObject *row = p.obj(QStringLiteral("InputDetail.bands.band.0"));
    QObject *menuObj = row ? row->property("typePicker").value<QObject *>() : nullptr;
    if (menuObj) QMetaObject::invokeMethod(menuObj, "rebuild");
    d.check(QStringLiteral("qp2: the row's type menu is built from the controller"), menuObj && menuObj->property("count").toInt() == 8);
}

void inputPage(Pages &p) {
    SmokeDriver &d = p.d;
    p.show("input", 0);
    const QString row = QStringLiteral("InputDetail.bands.band.0");
    if (!p.cc->isLinked(0)) p.cc->linkPair(0, 0);
    d.waitIdle();
    d.check(QStringLiteral("qp2: inputs 1/2 are linked by default"), p.cc->isLinked(0) && p.cc->linkedPartner(0) == 1);

    QObject *r = p.obj(row);
    d.check(QStringLiteral("qp2: the input's band rows exist"), r != nullptr);
    if (r) QMetaObject::invokeMethod(r, "chooseType", Q_ARG(QVariant, FILTER_PEAKING));
    d.waitIdle();
    d.check(QStringLiteral("qp2: choosing Peaking reaches the device"), p.s() && p.s()->filters[0][0].filter_type == FILTER_PEAKING);
    d.check(QStringLiteral("qp2: and is mirrored onto the linked input"), p.s() && p.s()->filters[1][0].filter_type == FILTER_PEAKING);

    p.type(row + QStringLiteral(".freq"), QStringLiteral("250"));
    p.type(row + QStringLiteral(".gain"), QStringLiteral("3.5"));
    p.type(row + QStringLiteral(".q"), QStringLiteral("2"));
    d.check(QStringLiteral("qp2: typed frequency, gain and Q are sent"),
            p.s() && near(p.s()->filters[0][0].freq, 250) && near(p.s()->filters[0][0].gain, 3.5) && near(p.s()->filters[0][0].q, 2));
    d.check(QStringLiteral("qp2: the typed values are mirrored"), p.s() && near(p.s()->filters[1][0].freq, 250));
    p.type(row + QStringLiteral(".freq"), QStringLiteral("abc"));
    d.check(QStringLiteral("qp2: a field that is not a number keeps the value"),
            p.s() && near(p.s()->filters[0][0].freq, 250) &&
                p.obj(row + QStringLiteral(".freq.field"))->property("text").toString() == QLatin1String("250.0"));
    const int before = p.device->settersRun();
    p.cc->setBandValue(0, 1, false, QStringLiteral("freq"), 500);
    p.cc->setBandValue(0, 1, false, QStringLiteral("gain"), -2);
    d.check(QStringLiteral("qp2: an edit shows at once, before the device answers"),
            near(p.cc->band(0, 1, false).value(QStringLiteral("gain")).toDouble(), -2));
    d.waitIdle();
    d.check(QStringLiteral("qp2: two quick edits of one band both land"),
            p.s() && near(p.s()->filters[0][1].freq, 500) && near(p.s()->filters[0][1].gain, -2) && p.device->settersRun() > before);
    p.cc->setBandType(0, 1, false, FILTER_LOWSHELF1);
    d.waitIdle();

    p.click(row + QStringLiteral(".bypass"));
    d.check(QStringLiteral("qp2: the bypass disc bypasses the band and its mirror"),
            p.s() && p.s()->filters[0][0].bypass && p.s()->filters[1][0].bypass);
    p.click(QStringLiteral("InputDetail.bands.enableAll"));
    d.check(QStringLiteral("qp2: Enable All"), p.s() && !p.s()->filters[0][0].bypass && !p.s()->filters[0][1].bypass);
    p.click(QStringLiteral("InputDetail.bands.bypassAll"));
    d.check(QStringLiteral("qp2: Bypass All on a PEQ list asks nothing"),
            p.s() && p.s()->filters[0][0].bypass && p.s()->filters[0][1].bypass && !p.s()->filters[0][2].bypass);
    QObject *bypassAll = p.obj(QStringLiteral("InputDetail.bands.bypassAll"));
    d.check(QStringLiteral("qp2: Bypass All is dimmed once everything is bypassed"), bypassAll && !bypassAll->property("enabled").toBool());
    p.click(QStringLiteral("InputDetail.bands.enableAll"));
    d.check(QStringLiteral("qp2: inputs have no Clear All in the list (the header has it)"),
            !p.obj(QStringLiteral("InputDetail.bands.clearAll"))->property("visible").toBool());

    // Preamp: field, mirror, then a mismatch when linking again
    p.type(QStringLiteral("InputDetail.preampValue"), QStringLiteral("-6"));
    d.check(QStringLiteral("qp2: the preamp field sets both linked inputs"),
            p.s() && near(p.s()->input_preamp_db[0], -6) && near(p.s()->input_preamp_db[1], -6));
    p.click(QStringLiteral("InputDetail.link"));
    d.check(QStringLiteral("qp2: Link unlinks without a question"), !p.cc->isLinked(0));
    p.type(QStringLiteral("InputDetail.preampValue"), QStringLiteral("-3"));
    d.check(QStringLiteral("qp2: unlinked, the preamp is this input's only"),
            p.s() && near(p.s()->input_preamp_db[0], -3) && near(p.s()->input_preamp_db[1], -6));
    p.click(QStringLiteral("InputDetail.link"));
    QObject *ask = p.obj(QStringLiteral("InputDetail.linkQuestion"));
    d.check(QStringLiteral("qp2: linking a mismatched pair asks which side to keep"), p.opened(ask));
    d.check(QStringLiteral("qp2: the question names the inputs and the trims"),
            ask && ask->property("title").toString() == QLatin1String("Inputs 1 and 2 don't match") &&
                ask->property("text").toString().contains(QLatin1String("different input trims")) &&
                ask->property("acceptText").toString() == QLatin1String("Keep IN1") &&
                ask->property("alternativeText").toString() == QLatin1String("Keep IN2"));
    d.spin(150);
    d.check(QStringLiteral("qp2: screenshot of the link question"), d.grab(p.w, QStringLiteral("window-input-link.png")));
    p.answer(ask, "reject");
    d.check(QStringLiteral("qp2: Cancel leaves the pair unlinked"), !p.cc->isLinked(0));
    p.click(QStringLiteral("InputDetail.link"));
    p.opened(ask);
    p.answer(ask, "alternative");
    d.check(QStringLiteral("qp2: Keep IN2 copies input 2 onto input 1 and links"),
            p.cc->isLinked(0) && p.s() && near(p.s()->input_preamp_db[0], -6));

    d.check(QStringLiteral("qp2: screenshot of the input page"), d.grab(p.w, QStringLiteral("window-input.png")));
    p.click(QStringLiteral("InputDetail.clear"));
    bool flat = p.s() != nullptr;
    for (int b = 0; flat && b < BANDS_PER_CHANNEL; ++b)
        flat = p.s()->filters[0][b].filter_type == FILTER_FLAT && p.s()->filters[1][b].filter_type == FILTER_FLAT;
    d.check(QStringLiteral("qp2: Clear 1/2 PEQ clears both halves"), flat);
    p.type(QStringLiteral("InputDetail.preampValue"), QStringLiteral("0"));
}

void outputPage(Pages &p) {
    SmokeDriver &d = p.d;
    p.show("output", 0);
    const int ch = p.cc->outputChannel(0);
    const QString row = QStringLiteral("OutputDetail.bands.band.0");

    // Linkwitz Transform: entering it starts flat, the panel drafts, Apply sends
    QObject *r = p.obj(row);
    if (r) QMetaObject::invokeMethod(r, "chooseType", Q_ARG(QVariant, FILTER_PEAKING));
    d.waitIdle();
    p.type(row + QStringLiteral(".freq"), QStringLiteral("40"));
    p.type(row + QStringLiteral(".gain"), QStringLiteral("6"));
    if (r) QMetaObject::invokeMethod(r, "chooseType", Q_ARG(QVariant, FILTER_LINKWITZ_TRANSFORM));
    d.waitIdle();
    d.check(QStringLiteral("qp2: a band turned Linkwitz starts flat (fp = f0)"),
            p.s() && p.s()->filters[ch][0].filter_type == FILTER_LINKWITZ_TRANSFORM && near(p.s()->filters[ch][0].gain, 40, 0.06));
    p.click(row + QStringLiteral(".linkwitz"));
    QObject *panel = r ? r->property("linkwitzPopover").value<QObject *>() : nullptr;
    d.check(QStringLiteral("qp2: the Linkwitz panel opens"), p.opened(panel));
    p.type(QStringLiteral("Linkwitz.fp"), QStringLiteral("20"), panel);
    QObject *state = p.obj(QStringLiteral("Linkwitz.state"), panel);
    QObject *boost = p.obj(QStringLiteral("Linkwitz.boost"), panel);
    d.check(QStringLiteral("qp2: a draft is not sent"), p.s() && near(p.s()->filters[ch][0].gain, 40, 0.06));
    d.check(QStringLiteral("qp2: the panel says the draft is not applied"),
            state && state->property("text").toString() == QLatin1String("Not applied yet"));
    d.check(QStringLiteral("qp2: the panel shows the implied boost"), boost && boost->property("text").toString() == QLatin1String("+12.0 dB"));
    d.spin(150);
    d.check(QStringLiteral("qp2: screenshot of the Linkwitz panel"), d.grab(p.w, QStringLiteral("window-linkwitz.png")));
    p.click(QStringLiteral("Linkwitz.apply"), panel);
    d.check(QStringLiteral("qp2: Apply sends the transform"), p.s() && near(p.s()->filters[ch][0].gain, 20, 0.06));
    d.check(QStringLiteral("qp2: and the panel says so"), state && state->property("text").toString() == QLatin1String("Applied"));
    if (panel) QMetaObject::invokeMethod(panel, "close");
    if (r) QMetaObject::invokeMethod(r, "chooseType", Q_ARG(QVariant, FILTER_PEAKING));
    d.waitIdle();
    d.check(QStringLiteral("qp2: leaving the Linkwitz Transform drops fp back to 0 dB"), p.s() && near(p.s()->filters[ch][0].gain, 0));

    // Clear All asks first
    p.click(QStringLiteral("OutputDetail.bands.clearAll"));
    QObject *clear = p.obj(QStringLiteral("OutputDetail.bands.clearQuestion"));
    d.check(QStringLiteral("qp2: Clear All asks"), p.opened(clear) && clear->property("title").toString() == QLatin1String("Clear All Bands?"));
    p.answer(clear, "reject");
    d.check(QStringLiteral("qp2: Cancel clears nothing"), p.s() && p.s()->filters[ch][0].filter_type == FILTER_PEAKING);
    p.click(QStringLiteral("OutputDetail.bands.clearAll"));
    p.opened(clear);
    p.answer(clear, "accept");
    d.check(QStringLiteral("qp2: Clear All resets the list"), p.s() && p.s()->filters[ch][0].filter_type == FILTER_FLAT);

    // Card: gain, delay (SL1), mute, routing, limiter
    p.type(QStringLiteral("OutputCard.gainValue"), QStringLiteral("-4"));
    d.check(QStringLiteral("qp2: the gain field sets the output gain"), p.s() && near(p.s()->output_gain_db[0], -4));
    // -- WP QPI1 -- a drag previews the gain on the graph, a release hands it back
    if (auto *gain = qobject_cast<QQuickItem *>(p.obj(QStringLiteral("OutputCard.gain")))) {
        GraphController *g = Registrar::instance<GraphController>();
        const int out0 = p.cc->outputChannel(0);
        const QPoint from = gain->mapToScene(QPointF(gain->width() * 0.5, gain->height() / 2)).toPoint();
        const QPoint to = gain->mapToScene(QPointF(gain->width() * 0.8, gain->height() / 2)).toPoint();
        QTest::mousePress(p.w, Qt::LeftButton, Qt::NoModifier, from);
        for (int i = 1; i <= 6; ++i) {
            QTest::mouseMove(p.w, from + (to - from) * i / 6);
            d.spin(20);
        }
        const double shown = g->outputGain(out0);
        d.check(QStringLiteral("qpi1: the graph follows the gain slider while it is held"),
                near(shown, gain->property("value").toDouble(), 0.01) && !near(shown, -4, 0.5));
        QTest::mouseRelease(p.w, Qt::LeftButton, Qt::NoModifier, to);
        d.waitIdle();
        d.check(QStringLiteral("qpi1: on release the graph takes the device's gain"),
                p.s() && near(g->outputGain(out0), p.s()->output_gain_db[0], 0.01) && !near(p.s()->output_gain_db[0], -4, 0.5));
        p.type(QStringLiteral("OutputCard.gainValue"), QStringLiteral("-4"));
    }
    QObject *delay = p.obj(QStringLiteral("OutputCard.delay"));
    if (delay) QMetaObject::invokeMethod(delay, "valueMoved", Q_ARG(double, 3.6));
    d.waitIdle();
    d.check(QStringLiteral("qp2: a delay drag sends whole milliseconds"), p.s() && near(p.s()->output_delay_ms[0], 4));
    p.type(QStringLiteral("OutputCard.delayValue"), QStringLiteral("2.5"));
    d.check(QStringLiteral("qp2: a typed delay is kept exactly"), p.s() && near(p.s()->output_delay_ms[0], 2.5, 1e-4));
    d.check(QStringLiteral("qp2: the delay slider ends at the device's line"),
            delay && near(delay->property("to").toDouble(), p.cc->maxDelayMs()) && p.cc->maxDelayMs() >= 21);
    int refused = 0;
    const QMetaObject::Connection c = QObject::connect(p.device, &DeviceModel::writeFailed, p.device, [&](const QString &) { ++refused; });
    p.type(QStringLiteral("OutputCard.delayValue"), QString::number(p.cc->maxDelayMs() + 10));
    QObject::disconnect(c);
    d.check(QStringLiteral("qp2: a delay past the line is refused by the core"),
            refused == 1 && p.s() && near(p.s()->output_delay_ms[0], 2.5, 1e-4));
    p.click(QStringLiteral("OutputCard.mute"));
    d.check(QStringLiteral("qp2: mute"), p.s() && p.s()->output_muted[0]);
    p.click(QStringLiteral("OutputCard.mute"));
    d.check(QStringLiteral("qp2: unmute"), p.s() && !p.s()->output_muted[0]);

    const bool routed = p.s() && p.s()->matrix_routing[1][0];
    p.click(QStringLiteral("OutputCard.route.1"));
    d.check(QStringLiteral("qp2: the routing row connects or disconnects input 2"), p.s() && p.s()->matrix_routing[1][0] != routed);
    if (!p.s()->matrix_routing[1][0]) p.click(QStringLiteral("OutputCard.route.1"));
    p.type(QStringLiteral("OutputCard.routeGain.1"), QStringLiteral("-2.5"));
    p.click(QStringLiteral("OutputCard.routeInvert.1"));
    d.check(QStringLiteral("qp2: crosspoint gain and invert"),
            p.s() && near(p.s()->matrix_gain[1][0], -2.5) && p.s()->matrix_invert[1][0]);
    p.click(QStringLiteral("OutputCard.routeInvert.1"));

    d.check(QStringLiteral("qp2: the demo (V32) has the limiter cell"),
            p.cc->limiterSupported() && p.obj(QStringLiteral("OutputCard.limiter"))->property("visible").toBool());
    const bool lim = p.s() && p.s()->limiter_enabled[0];
    p.click(QStringLiteral("OutputCard.limiter"));
    d.check(QStringLiteral("qp2: a click toggles the output limiter"), p.s() && p.s()->limiter_enabled[0] != lim);
    int asked = -1;
    const QMetaObject::Connection l = QObject::connect(p.cc, &ChannelController::limiterSettingsRequested, p.cc, [&](int o) { asked = o; });
    QMetaObject::invokeMethod(p.obj(QStringLiteral("OutputCard.limiter")), "secondaryClicked");
    QObject::disconnect(l);
    d.check(QStringLiteral("qp2: a right-click asks for the limiter's settings"), asked == 0);
    d.check(QStringLiteral("qp2: the limiter meter polls while an output page shows"),
            d.waitFor(2000, [&]() { return p.cc->limiterReduction().size() == MAX_OUTPUTS; }));
    d.check(QStringLiteral("qp2: screenshot of the output page"), d.grab(p.w, QStringLiteral("window-output.png")));
}

void crossoverTab(Pages &p) {
    SmokeDriver &d = p.d;
    p.show("output", 0);
    const int ch = p.cc->outputChannel(0);
    d.check(QStringLiteral("qp2: the demo has crossovers"), p.cc->crossoverSupported());
    p.click(QStringLiteral("OutputDetail.bands.tab.xo"));
    QObject *xo = p.obj(QStringLiteral("OutputDetail.crossover"));
    d.check(QStringLiteral("qp2: the XO tab shows the crossover list"), xo && xo->property("visible").toBool());
    d.check(QStringLiteral("qp2: the graph is told the XO tab shows"), p.cc->crossoverTabShown());
    const QString row = QStringLiteral("OutputDetail.crossover.band.0");
    QObject *family = p.obj(row + QStringLiteral(".family"));
    if (family) QMetaObject::invokeMethod(family, "activated", Q_ARG(int, 1));
    d.waitIdle();
    d.check(QStringLiteral("qp2: Linkwitz-Riley from Off is LR2 low pass"), p.s() && p.s()->xover[ch][0].filter_type == 32);
    QObject *slope = p.obj(row + QStringLiteral(".slope"));
    if (slope) QMetaObject::invokeMethod(slope, "activated", Q_ARG(int, 1));
    d.waitIdle();
    QObject *dir = p.obj(row + QStringLiteral(".direction"));
    if (dir) QMetaObject::invokeMethod(dir, "activated", Q_ARG(int, 1));
    d.waitIdle();
    d.check(QStringLiteral("qp2: slope 24 dB/oct and high pass give LR4 high pass"), p.s() && p.s()->xover[ch][0].filter_type == 35);
    if (family) QMetaObject::invokeMethod(family, "activated", Q_ARG(int, 2));
    d.waitIdle();
    d.check(QStringLiteral("qp2: Butterworth keeps order 4 and high pass"), p.s() && p.s()->xover[ch][0].filter_type == 47);
    p.type(row + QStringLiteral(".freq"), QStringLiteral("80"));
    d.check(QStringLiteral("qp2: crossover frequency"), p.s() && near(p.s()->xover[ch][0].freq, 80));
    d.check(QStringLiteral("qp2: screenshot of the XO tab"), d.grab(p.w, QStringLiteral("window-output-xo.png")));

    p.click(QStringLiteral("OutputDetail.crossover.bypassAll"));
    QObject *ask = p.obj(QStringLiteral("OutputDetail.crossover.bypassQuestion"));
    d.check(QStringLiteral("qp2: bypassing crossovers asks first"),
            p.opened(ask) && ask->property("title").toString() == QLatin1String("Bypass this output's crossovers?"));
    p.answer(ask, "reject");
    d.check(QStringLiteral("qp2: Cancel bypasses nothing"), p.s() && !p.s()->xover[ch][0].bypass);
    p.click(QStringLiteral("OutputDetail.crossover.bypassAll"));
    p.opened(ask);
    p.answer(ask, "accept");
    d.check(QStringLiteral("qp2: Bypass All bypasses the crossover bands"), p.s() && p.s()->xover[ch][0].bypass);
    p.click(QStringLiteral("OutputDetail.crossover.enableAll"));
    d.check(QStringLiteral("qp2: Enable All asks nothing"), p.s() && !p.s()->xover[ch][0].bypass);
    p.click(QStringLiteral("OutputDetail.crossover.tab.peq"));
    d.check(QStringLiteral("qp2: back on the PEQ tab"), !xo->property("visible").toBool() && !p.cc->crossoverTabShown());
}

void enableQuestions(Pages &p) {
    SmokeDriver &d = p.d;
    const int pdm = p.cc->pdmOutput();
    d.check(QStringLiteral("qp2: the demo has a PDM output after the EQ outputs"), pdm >= 4);
    if (pdm < 4) return;
    p.cc->requestOutputEnable(pdm, false);
    d.waitIdle();
    p.cc->requestOutputEnable(2, true);
    d.waitIdle();
    d.check(QStringLiteral("qp2: with PDM off, output 3 enables without a question"), p.s() && p.s()->output_enabled[2]);

    p.show("output", pdm);
    QObject *ask = p.obj(QStringLiteral("OutputDetail.core1Question"));
    p.click(QStringLiteral("OutputDetail.enable"));
    d.check(QStringLiteral("qp2: enabling PDM asks first"), p.opened(ask));
    d.check(QStringLiteral("qp2: and names the outputs it disables"),
            ask && ask->property("text").toString() == QStringLiteral("Outputs 3-%1 will be disabled. Are you sure?").arg(pdm) &&
                ask->property("acceptText").toString() == QLatin1String("Enable PDM"));
    d.check(QStringLiteral("qp2: nothing is sent before the answer"), p.s() && !p.s()->output_enabled[pdm]);
    d.check(QStringLiteral("qp2: the switch shows the device's state meanwhile"),
            !p.obj(QStringLiteral("OutputDetail.enable"))->property("checked").toBool());
    d.check(QStringLiteral("qp2: screenshot of the PDM question"), d.grab(p.w, QStringLiteral("window-output-pdm.png")));
    p.answer(ask, "accept");
    bool eqOff = p.s() != nullptr;
    for (int o = 2; eqOff && o < pdm; ++o) eqOff = !p.s()->output_enabled[o];
    d.check(QStringLiteral("qp2: Enable PDM turns the EQ outputs off and PDM on"), eqOff && p.s()->output_enabled[pdm]);

    p.show("output", 2);
    p.click(QStringLiteral("OutputDetail.enable"));
    d.check(QStringLiteral("qp2: enabling output 3 under PDM asks"),
            p.opened(ask) && ask->property("text").toString() == QLatin1String("The PDM output will be disabled. Are you sure?"));
    p.answer(ask, "reject");
    d.check(QStringLiteral("qp2: Cancel changes nothing"), p.s() && !p.s()->output_enabled[2] && p.s()->output_enabled[pdm]);
    p.click(QStringLiteral("OutputDetail.enable"));
    p.opened(ask);
    p.answer(ask, "accept");
    d.check(QStringLiteral("qp2: Disable PDM turns PDM off and output 3 on"), p.s() && p.s()->output_enabled[2] && !p.s()->output_enabled[pdm]);
    p.click(QStringLiteral("OutputDetail.enable"));
    d.check(QStringLiteral("qp2: disabling never asks"), p.s() && !p.s()->output_enabled[2] && !p.obj(QStringLiteral("OutputDetail.core1Question"))->property("visible").toBool());
}

void readOnly(Pages &p) {
    SmokeDriver &d = p.d;
    p.d.app().core->worker()->post([](FfiCore *c) { dspi_demo_open_wire(c, 17); });
    p.device->refresh();
    d.check(QStringLiteral("qp2: the V17 recording opens read-only"), d.waitFor(5000, [&]() { return p.device->readOnly(); }));
    d.waitIdle();
    p.show("output", 0);
    const QVariantList types = p.cc->availableTypes(true);
    d.check(QStringLiteral("qp2: V17 has no Linkwitz Transform and no first-order passes"),
            !types.contains(FILTER_LINKWITZ_TRANSFORM) && !types.contains(FILTER_LOWPASS1) && types.contains(FILTER_LOWSHELF1));
    d.check(QStringLiteral("qp2: V17 hides the limiter cell"),
            !p.cc->limiterSupported() && !p.obj(QStringLiteral("OutputCard.limiter"))->property("visible").toBool());
    d.check(QStringLiteral("qp2: read-only disables the card and the rows"),
            !p.obj(QStringLiteral("OutputCard"))->property("enabled").toBool() &&
                !p.obj(QStringLiteral("OutputDetail.bands.band.0"))->property("enabled").toBool() &&
                !p.obj(QStringLiteral("OutputDetail.bands.clearAll"))->property("enabled").toBool());
    d.check(QStringLiteral("qp2: the PEQ / XO tabs stay enabled"), p.obj(QStringLiteral("OutputDetail.bands.tab.xo"))->property("enabled").toBool());
    const int before = p.device->settersRun();
    p.cc->setOutputGain(0, -10);
    p.cc->setBandValue(p.cc->outputChannel(0), 0, false, QStringLiteral("freq"), 300);
    d.waitIdle();
    d.check(QStringLiteral("qp2: read-only sends nothing"), p.device->settersRun() == before);
    d.check(QStringLiteral("qp2: screenshot of a read-only output page"), d.grab(p.w, QStringLiteral("window-output-readonly.png")));
}

void channelSection(SmokeDriver &d) {
    auto *w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("smoke.channel")));
    d.check(QStringLiteral("qp2: the channel pages' window opens"), w && d.waitFor(3000, [&]() { return w->isVisible(); }));
    auto *cc = Registrar::instance<ChannelController>();
    d.check(QStringLiteral("qp2: the channel controller is registered"), cc != nullptr);
    if (!w || !cc) return;
    d.waitIdle();
    Pages p{ d, w, cc, d.app().device };
    d.check(QStringLiteral("qp2: the demo is writable"), cc->writable() && cc->numInputs() >= 2 && cc->numOutputs() >= 5);

    filterTypes(p);
    inputPage(p);
    outputPage(p);
    crossoverTab(p);
    enableQuestions(p);
    readOnly(p);

    // Back to the demo as it opens, for the sections after this one
    d.app().device->openDemo();
    d.check(QStringLiteral("qp2: the demo reopens writable"), d.waitFor(5000, [&]() { return !d.app().device->readOnly() && d.app().device->connected(); }));
    d.waitIdle();
    w->close();
}

const SmokeDriver::Section channel("qp2-channel", 20, &channelSection);

} // namespace
