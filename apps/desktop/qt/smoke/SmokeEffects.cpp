// -- WP QP7 -- The four effect windows on the demo device: crossfeed with the
// pair map (Default, K340, Custom, the mask question, the editor's refusal of
// an invalid map, hidden on a stall) and the dimmed preset rows; loudness,
// the leveller and psychoacoustic bass: switches, parameters, masks and
// their menus, the curves' maths. Every write is checked in the device's state.
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "EffectsController.h"
#include "ExtController.h"
#include "Shell.h"
#include "SidebarController.h"
#include "Snapshot.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <QVariantMap>
#include <cmath>
#include <functional>

namespace {

using Fx = EffectsController;

bool near(double a, double b, double tolerance = 0.01) { return std::fabs(a - b) <= tolerance; }

struct Win {
    SmokeDriver &d;
    QQuickWindow *w = nullptr;

    QQuickItem *item(const QString &name) const { return w ? d.item(name, w->contentItem()) : nullptr; }
    // A menu row (not in the item tree while its menu is closed)
    QObject *row(const QString &name) const { return w ? w->findChild<QObject *>(name) : nullptr; }
    // Scrolls the window's body until `it` is in view, as a person would
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
    void click(const QString &name) const {
        QQuickItem *it = item(name);
        if (!it || !w) return;
        reveal(it);
        QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier,
                          it->mapToScene(QPointF(it->width() / 2, it->height() / 2)).toPoint());
        d.spin(60);
    }
    void trigger(const QString &name) const {
        if (QObject *o = row(name)) QMetaObject::invokeMethod(o, "triggered");
        d.spin(30);
    }
    bool shows(const QString &name) const {
        QQuickItem *it = item(name);
        return it && it->isVisible();
    }
};

const DspState *state(SmokeDriver &d) { return d.app().device->state(); }

bool waitState(SmokeDriver &d, const std::function<bool(const DspState &)> &done) {
    return d.waitFor(3000, [&]() {
        d.waitIdle(1000);
        const DspState *s = state(d);
        return s && done(*s);
    });
}

QByteArray deviceMap(SmokeDriver &d) {
    QByteArray map;
    bool done = false;
    d.app().core->worker()->call(
        d.app().device,
        [](FfiCore *c) {
            QByteArray m(4, '\0');
            return dspi_fetch_crossfeed_pair_map(c, reinterpret_cast<uint8_t *>(m.data()), 4) ? m : QByteArray();
        },
        [&](const QByteArray &m) {
            map = m;
            done = true;
        });
    d.waitFor(3000, [&]() { return done; });
    return map;
}

// -- WP P1XF -- the device's custom crossfeed pair, which the state shows only
// under Custom: the export carries it whatever the preset
std::pair<double, double> exportedCustomPair(SmokeDriver &d) {
    QByteArray doc;
    bool done = false;
    d.app().core->worker()->call(
        d.app().device,
        [](FfiCore *c) {
            return readSized([&](char *b, uint32_t n) { return dspi_config_export(c, "smoke", "0", 0, b, n); });
        },
        [&](const QByteArray &text) {
            doc = text;
            done = true;
        });
    d.waitFor(3000, [&]() { return done; });
    const QJsonObject cf = QJsonDocument::fromJson(doc).object().value(QStringLiteral("crossfeed")).toObject();
    return { cf.value(QStringLiteral("freqHz")).toDouble(-1), cf.value(QStringLiteral("feedDb")).toDouble(-1) };
}

Win open(SmokeDriver &d, const char *action, const char *type) {
    Win win{ d };
    win.w = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QString::fromLatin1(action)));
    d.check(QStringLiteral("qp7: %1 opens the %2").arg(QLatin1String(action), QLatin1String(type)),
            win.w && d.waitFor(3000, [&]() { return win.w->isVisible(); }) && win.w->objectName() == QLatin1String(type));
    if (win.w) d.spin(150);
    return win;
}

// ---- The maths, against the Mac's formulas ----

void maths(SmokeDriver &d) {
    // The BS2B filter at DC: the opposite path is G = 1 / (1 + 10^(feed/20)), the direct one 1 - G
    const double g = 1.0 / (1.0 + std::pow(10.0, 4.5 / 20.0));
    d.check(QStringLiteral("qp7: crossfeed's opposite path is G below the cutoff"),
            near(Fx::crossfeedOppositeDb(700, 4.5, 20), 20 * std::log10(g), 0.05));
    d.check(QStringLiteral("qp7: crossfeed's direct path is 1 - G below the cutoff"),
            near(Fx::crossfeedDirectDb(700, 4.5, 20), 20 * std::log10(1 - g), 0.05));
    d.check(QStringLiteral("qp7: crossfeed's opposite path falls above the cutoff"),
            Fx::crossfeedOppositeDb(700, 4.5, 10000) < Fx::crossfeedOppositeDb(700, 4.5, 100) - 10);
    d.check(QStringLiteral("qp7: crossfeed's direct path rises to 0 dB"), near(Fx::crossfeedDirectDb(700, 4.5, 20000), 0, 0.5));

    int k1 = -1, k20 = -1;
    for (int i = 0; i < Fx::isoRows(); ++i) {
        if (Fx::isoFreq(i) == 1000) k1 = i;
        if (Fx::isoFreq(i) == 20) k20 = i;
    }
    d.check(QStringLiteral("qp7: the ISO table is the Mac's 30 rows, 20 Hz to 16 kHz"),
            Fx::isoRows() == 30 && k20 == 0 && Fx::isoFreq(29) == 16000 && k1 == 17);
    d.check(QStringLiteral("qp7: loudness leaves 1 kHz alone"), std::fabs(Fx::loudnessCompensationDb(83, 100, k1)) < 0.5);
    const double bass = Fx::loudnessCompensationDb(83, 100, k20);
    d.check(QStringLiteral("qp7: loudness lifts 20 Hz at -40 dB"), bass > 10 && bass < 40);
    d.check(QStringLiteral("qp7: loudness scales with intensity"),
            near(Fx::loudnessCompensationDb(83, 200, k20), 2 * bass, 1e-6) && near(Fx::loudnessCompensationDb(83, 0, k20), 0));

    Fx *fx = Registrar::instance<Fx>();
    const QVariantMap lg = fx->loudnessGraph(83, 100);
    const QVariantList curve = lg.value(QStringLiteral("curve")).toList();
    bool inside = curve.size() == 60;
    for (const QVariant &v : curve) inside = inside && v.toDouble() >= 0 && v.toDouble() <= 1;
    d.check(QStringLiteral("qp7: the loudness curve has a point per ISO row, all on the graph"), inside);
    const QVariantList grid = lg.value(QStringLiteral("gridDb")).toList();
    d.check(QStringLiteral("qp7: the loudness graph's grid is in 5 or 10 dB steps"),
            grid.size() >= 2 && (std::abs(grid.at(1).toInt() - grid.at(0).toInt()) == 5 ||
                                 std::abs(grid.at(1).toInt() - grid.at(0).toInt()) == 10));
    const QVariantMap cg = fx->crossfeedGraph(700, 4.5);
    d.check(QStringLiteral("qp7: the crossfeed graph has two curves of 100 points"),
            cg.value(QStringLiteral("direct")).toList().size() == 200 &&
                cg.value(QStringLiteral("crossfeed")).toList().size() == 200);
    const QVariantMap sk = fx->psybassSketch(80, 12, -60);
    d.check(QStringLiteral("qp7: the psybass sketch: harmonics full at +12 dB, original gone at -60 dB"),
            near(sk.value(QStringLiteral("harmonics")).toDouble(), 1) && near(sk.value(QStringLiteral("original")).toDouble(), 0) &&
                sk.value(QStringLiteral("fc4")).toDouble() > sk.value(QStringLiteral("fc")).toDouble());
    d.check(QStringLiteral("qp7: the frequency axis runs from 20 Hz to 20 kHz"),
            near(Fx::freqX(20), 0) && near(Fx::freqX(20000), 1) && near(Fx::freqX(632.456), 0.5, 0.001));
    d.check(QStringLiteral("qp7: the crossfeed presets are the Mac's four"),
            Fx::crossfeedPresets().size() == 4 &&
                Fx::crossfeedPresets().at(2).toMap().value(QStringLiteral("freq")).toDouble() == 650);
    d.check(QStringLiteral("qp7: psybass has the Mac's four starting points"), Fx::psybassStartingPoints().size() == 4);
}

// ---- Crossfeed ----

void crossfeed(SmokeDriver &d) {
    Fx *fx = Registrar::instance<Fx>();
    ExtController *ext = Registrar::instance<ExtController>();
    Win win = open(d, "tools.crossfeed", "CrossfeedWindow");
    if (!win.w) return;
    d.check(QStringLiteral("qp7: the crossfeed window is the Mac's size"),
            win.w->width() == 780 && win.w->height() == 470 && win.w->minimumWidth() == 740 && win.w->minimumHeight() == 460);
    d.check(QStringLiteral("qp7: the window's title"), win.w->title() == QLatin1String("Crossfeed"));
    const DspState *s0 = state(d);
    if (!s0) return;
    const bool wasOn = s0->crossfeed_enabled;
    const int wasPreset = s0->crossfeed_preset;
    const int wasMask = s0->crossfeed_output_pair_mask;
    const double wasCustomFreq = exportedCustomPair(d).first; // -- WP P1XF --

    // The switch, and the quick strip following it
    win.click(QStringLiteral("CrossfeedWindow.header.switch"));
    d.check(QStringLiteral("qp7: the crossfeed switch writes"), waitState(d, [&](const DspState &s) { return s.crossfeed_enabled != wasOn; }));
    d.check(QStringLiteral("qp7: the quick strip follows the window's switch"),
            Registrar::instance<SidebarController>()->crossfeedOn() == !wasOn);
    if (wasOn) {
        win.click(QStringLiteral("CrossfeedWindow.header.switch"));
        waitState(d, [](const DspState &s) { return s.crossfeed_enabled; });
    }
    d.check(QStringLiteral("qp7: the graph draws with crossfeed on"),
            win.item(QStringLiteral("CrossfeedWindow.graph")) &&
                win.item(QStringLiteral("CrossfeedWindow.graph"))->property("active").toBool());

    // Presets: the rows dim and show the preset's own values
    win.click(QStringLiteral("CrossfeedWindow.preset.2"));
    d.check(QStringLiteral("qp7: a preset row chooses the preset"), waitState(d, [](const DspState &s) { return s.crossfeed_preset == 2; }));
    QQuickItem *params = win.item(QStringLiteral("CrossfeedWindow.parameters"));
    QQuickItem *freqValue = win.item(QStringLiteral("CrossfeedWindow.freq.value"));
    QQuickItem *feedValue = win.item(QStringLiteral("CrossfeedWindow.feed.value"));
    d.check(QStringLiteral("qp7: a preset dims the parameter rows"), params && params->opacity() < 0.6);
    d.check(QStringLiteral("qp7: the dimmed rows show the preset's own values"),
            freqValue && feedValue && freqValue->property("text").toString() == QLatin1String("650.0") &&
                feedValue->property("text").toString() == QLatin1String("9.5"));
    // -- WP P1XF -- the state gives Meier's values; the device keeps its custom pair
    d.check(QStringLiteral("p1xf: the state gives the preset's own values"),
            near(fx->crossfeedFreq(), 650) && near(fx->crossfeedFeed(), 9.5));
    d.check(QStringLiteral("qp7: the device keeps its custom values meanwhile"), near(exportedCustomPair(d).first, wasCustomFreq));
    d.check(QStringLiteral("qp7: screenshot of crossfeed with a preset"), d.grab(win.w, QStringLiteral("window-crossfeed-preset.png")));

    // A drag's value sends the number only; a release makes the preset Custom
    QQuickItem *freqRow = win.item(QStringLiteral("CrossfeedWindow.freq"));
    if (freqRow) QMetaObject::invokeMethod(freqRow, "live", Q_ARG(double, 820.0));
    // -- WP P1XF -- under Meier the state keeps showing 650; the 820 is the device's custom value
    d.check(QStringLiteral("qp7: a live value reaches the device"),
            d.waitFor(3000, [&]() { return near(exportedCustomPair(d).first, 820); }));
    d.check(QStringLiteral("p1xf: a live value under a preset leaves the preset's values shown"),
            near(state(d)->crossfeed_freq, 650) && near(fx->crossfeedShownFreq(), 650));
    d.check(QStringLiteral("qp7: a live value leaves the preset"), state(d)->crossfeed_preset == 2);
    if (freqRow) QMetaObject::invokeMethod(freqRow, "commit", Q_ARG(double, 900.0));
    d.check(QStringLiteral("qp7: a released value makes the preset Custom"),
            waitState(d, [](const DspState &s) { return s.crossfeed_preset == 3 && near(s.crossfeed_freq, 900); }));
    d.check(QStringLiteral("qp7: Custom shows the rows at full strength"), params && params->opacity() > 0.99);
    d.check(QStringLiteral("qp7: Custom shows the device's values"),
            freqValue && freqValue->property("text").toString() == QLatin1String("900.0"));
    if (freqRow) QMetaObject::invokeMethod(freqRow, "commit", Q_ARG(double, 5000.0));
    d.check(QStringLiteral("qp7: a typed frequency is held to 2000 Hz"),
            waitState(d, [](const DspState &s) { return near(s.crossfeed_freq, 2000); }));
    QQuickItem *feedRow = win.item(QStringLiteral("CrossfeedWindow.feed"));
    if (feedRow) QMetaObject::invokeMethod(feedRow, "commit", Q_ARG(double, 7.5));
    d.check(QStringLiteral("qp7: the feed level writes"), waitState(d, [](const DspState &s) { return near(s.crossfeed_feed, 7.5); }));

    const bool itd = state(d)->crossfeed_itd;
    win.click(QStringLiteral("CrossfeedWindow.itd"));
    d.check(QStringLiteral("qp7: the interaural delay switch writes"), waitState(d, [&](const DspState &s) { return s.crossfeed_itd != itd; }));
    win.click(QStringLiteral("CrossfeedWindow.itd"));
    waitState(d, [&](const DspState &s) { return s.crossfeed_itd == itd; });

    // The pair map
    d.check(QStringLiteral("qp7: the demo device lists the pair map"),
            d.waitFor(3000, [&]() { return ext->known() && ext->hasPairMap(); }) &&
                ext->supported().contains(QVariant(1)));
    d.check(QStringLiteral("qp7: the pair map shows"), win.shows(QStringLiteral("CrossfeedWindow.pairMap")));
    d.check(QStringLiteral("qp7: the pairs are chips in two columns, named by their outputs"),
            win.shows(QStringLiteral("CrossfeedWindow.pairChips")) &&
                win.item(QStringLiteral("CrossfeedWindow.pair.0")) &&
                win.item(QStringLiteral("CrossfeedWindow.pair.3")));
    QObject *map = win.item(QStringLiteral("CrossfeedWindow.pairMap"));
    const auto pick = [&](const char *key) {
        if (map) QMetaObject::invokeMethod(map, "pick", Q_ARG(QVariant, QString::fromLatin1(key)));
        d.spin(30);
    };
    pick("default");
    d.check(QStringLiteral("qp7: Default writes the default pairing"),
            d.waitFor(3000, [&]() { return ext->choice() == QLatin1String("default") && !ext->busy(); }) &&
                deviceMap(d) == QByteArray(4, '\0'));
    QQuickItem *chip0 = win.item(QStringLiteral("CrossfeedWindow.pair.0"));
    d.check(QStringLiteral("qp7: a default chip names outputs 1 and 2"),
            chip0 && chip0->property("text").toString() == QLatin1String("1: Out 1 / Out 2"));

    // Factory mask: pair 1 only, so the K340's question shows
    fx->setCrossfeedPairMask(0x01);
    waitState(d, [](const DspState &s) { return s.crossfeed_output_pair_mask == 0x01; });
    win.trigger(QStringLiteral("CrossfeedWindow.pairMap.choice.k340"));
    d.check(QStringLiteral("qp7: the Outputs menu's K340 row writes the K340 map"),
            d.waitFor(3000, [&]() { return ext->choice() == QLatin1String("k340") && !ext->busy(); }));
    QByteArray k340(4, '\0');
    dspi_crossfeed_k340_pair_map(reinterpret_cast<uint8_t *>(k340.data()), 4);
    d.check(QStringLiteral("qp7: the device reports the K340 map"), deviceMap(d) == ExtController::normalized(k340));
    d.check(QStringLiteral("qp7: the K340 chips name their outputs"),
            chip0 && chip0->property("text").toString() == QLatin1String("1: Out 3 / Out 1"));
    d.check(QStringLiteral("qp7: the menu shows the K340 choice"),
            win.item(QStringLiteral("CrossfeedWindow.pairMap.menu")) &&
                win.item(QStringLiteral("CrossfeedWindow.pairMap.menu"))->property("text").toString() ==
                    QLatin1String("One DAC per ear (K340)"));
    d.check(QStringLiteral("qp7: the K340 asks to crossfeed both pairs"),
            d.waitFor(1000, [&]() { return win.shows(QStringLiteral("CrossfeedWindow.pairMap.offer")); }));
    d.check(QStringLiteral("qp7: the question writes nothing by itself"), state(d)->crossfeed_output_pair_mask == 0x01);
    d.check(QStringLiteral("qp7: screenshot of the K340 map"), d.grab(win.w, QStringLiteral("window-crossfeed-k340.png")));
    win.click(QStringLiteral("CrossfeedWindow.pairMap.offer.enable"));
    d.check(QStringLiteral("qp7: Enable Pairs 1 and 2 adds the K340's pairs"),
            waitState(d, [](const DspState &s) { return s.crossfeed_output_pair_mask == (0x01 | ExtController::k340PairMask()); }));
    d.check(QStringLiteral("qp7: and the question goes"), !win.shows(QStringLiteral("CrossfeedWindow.pairMap.offer")));

    // The custom editor
    win.trigger(QStringLiteral("CrossfeedWindow.pairMap.choice.custom"));
    d.check(QStringLiteral("qp7: Custom opens the editor on the current pairs"),
            d.waitFor(1000, [&]() { return win.shows(QStringLiteral("CrossfeedWindow.pairMap.editor")); }) &&
                map && map->property("draft").toList().size() == ext->pairCount());
    d.check(QStringLiteral("qp7: the window says it is editing"), win.w->property("editingPairs").toBool());
    // One output twice in a pair
    QMetaObject::invokeMethod(map, "setDraft", Q_ARG(QVariant, 0), Q_ARG(QVariant, QStringLiteral("right")), Q_ARG(QVariant, 2));
    d.spin(30);
    d.check(QStringLiteral("qp7: a pair on one output twice is refused"),
            !map->property("draftValid").toBool() && win.shows(QStringLiteral("CrossfeedWindow.pairMap.editor.warning")) &&
                !win.item(QStringLiteral("CrossfeedWindow.pairMap.editor.apply"))->isEnabled());
    d.check(QStringLiteral("qp7: the controller refuses it too, sending nothing"),
            !ext->applyPairs(map->property("draft").toList()) && !ext->busy());
    d.check(QStringLiteral("qp7: screenshot of the editor's refusal"), d.grab(win.w, QStringLiteral("window-crossfeed-editor-short.png")));
    // Outputs 2/1, 4/3 and the rest default
    QMetaObject::invokeMethod(map, "setDraft", Q_ARG(QVariant, 0), Q_ARG(QVariant, QStringLiteral("left")), Q_ARG(QVariant, 1));
    QMetaObject::invokeMethod(map, "setDraft", Q_ARG(QVariant, 0), Q_ARG(QVariant, QStringLiteral("right")), Q_ARG(QVariant, 0));
    QMetaObject::invokeMethod(map, "setDraft", Q_ARG(QVariant, 1), Q_ARG(QVariant, QStringLiteral("left")), Q_ARG(QVariant, 3));
    QMetaObject::invokeMethod(map, "setDraft", Q_ARG(QVariant, 1), Q_ARG(QVariant, QStringLiteral("right")), Q_ARG(QVariant, 2));
    for (int p = 2; p < ext->pairCount(); ++p) {
        QMetaObject::invokeMethod(map, "setDraft", Q_ARG(QVariant, p), Q_ARG(QVariant, QStringLiteral("left")), Q_ARG(QVariant, 2 * p));
        QMetaObject::invokeMethod(map, "setDraft", Q_ARG(QVariant, p), Q_ARG(QVariant, QStringLiteral("right")), Q_ARG(QVariant, 2 * p + 1));
    }
    d.spin(30);
    d.check(QStringLiteral("qp7: swapped sides are a valid map"), map->property("draftValid").toBool());
    d.check(QStringLiteral("qp7: screenshot of the editor"), d.grab(win.w, QStringLiteral("window-crossfeed-editor.png")));
    win.click(QStringLiteral("CrossfeedWindow.pairMap.editor.apply"));
    d.check(QStringLiteral("qp7: Apply writes the map and closes the editor"),
            d.waitFor(3000, [&]() { return ext->choice() == QLatin1String("custom") && !ext->busy(); }) &&
                !win.w->property("editingPairs").toBool());
    const QByteArray custom = deviceMap(d);
    d.check(QStringLiteral("qp7: the device reads back the custom map"),
            custom.size() == 4 && uint8_t(custom.at(0)) == 0x01 && uint8_t(custom.at(1)) == 0x23 && custom.at(2) == 0 && custom.at(3) == 0);
    d.check(QStringLiteral("qp7: the menu says Custom"),
            win.item(QStringLiteral("CrossfeedWindow.pairMap.menu"))->property("text").toString() == QLatin1String("Custom"));

    // A notification brings a map changed elsewhere: read it again
    pick("default");
    d.check(QStringLiteral("qp7: back to the default pairing"),
            d.waitFor(3000, [&]() { return ext->choice() == QLatin1String("default") && !ext->busy(); }));

    // The output pair chips and their menu
    win.click(QStringLiteral("CrossfeedWindow.pair.2"));
    d.check(QStringLiteral("qp7: a pair chip toggles its pair"),
            waitState(d, [](const DspState &s) { return (s.crossfeed_output_pair_mask & 0x04) != 0; }));
    d.check(QStringLiteral("qp7: the chip shows it"),
            win.item(QStringLiteral("CrossfeedWindow.pair.2"))->property("on").toBool());
    win.trigger(QStringLiteral("CrossfeedWindow.pairs.none"));
    d.check(QStringLiteral("qp7: Presets > None clears the pairs"),
            waitState(d, [](const DspState &s) { return s.crossfeed_output_pair_mask == 0; }));
    win.trigger(QStringLiteral("CrossfeedWindow.pairs.all"));
    d.check(QStringLiteral("qp7: Presets > All pairs sets every pair of the platform"),
            waitState(d, [&](const DspState &s) { return s.crossfeed_output_pair_mask == fx->allPairsMask(); }) &&
                fx->allPairsMask() == 0x0F);

    // Hidden on a stall: what stock firmware's probe leaves
    pick("custom");
    ext->setStatusOverride(R"({"ok":true,"supported":[],"hasPairMap":false,"hasCoreVreg":false})");
    d.check(QStringLiteral("qp7: a stall hides the pair map"),
            d.waitFor(2000, [&]() { return !ext->hasPairMap() && !ext->busy(); }) &&
                !win.shows(QStringLiteral("CrossfeedWindow.pairMap")));
    d.check(QStringLiteral("qp7: a stall closes the editor"), !win.w->property("editingPairs").toBool());
    d.check(QStringLiteral("qp7: a stall leaves the plain chips"),
            win.shows(QStringLiteral("CrossfeedWindow.plainPairChips")) &&
                win.item(QStringLiteral("CrossfeedWindow.pair.0"))->property("text").toString() == QLatin1String("1"));
    d.check(QStringLiteral("qp7: a stall refuses a write"), !ext->applyChoice(QStringLiteral("k340")));
    d.check(QStringLiteral("qp7: screenshot of crossfeed on stock firmware"), d.grab(win.w, QStringLiteral("window-crossfeed-stock.png")));
    ext->setStatusOverride(QByteArray());
    d.check(QStringLiteral("qp7: the pair map comes back with the probe"),
            d.waitFor(2000, [&]() { return ext->hasPairMap() && !ext->busy(); }) && win.shows(QStringLiteral("CrossfeedWindow.pairMap")));
    d.check(QStringLiteral("qp7: screenshot of crossfeed"), d.grab(win.w, QStringLiteral("window-crossfeed.png")));

    // As it was
    fx->setCrossfeedPairMask(wasMask);
    fx->setCrossfeedFreq(wasCustomFreq, true); // -- WP P1XF --
    fx->setCrossfeedPreset(wasPreset);
    fx->setEnabled(QStringLiteral("crossfeed"), wasOn);
    waitState(d, [&](const DspState &s) { return s.crossfeed_preset == wasPreset && s.crossfeed_enabled == wasOn; });
    win.w->close();
}

// ---- Loudness ----

void loudness(SmokeDriver &d) {
    Fx *fx = Registrar::instance<Fx>();
    Win win = open(d, "tools.loudness", "LoudnessWindow");
    if (!win.w || !state(d)) return;
    d.check(QStringLiteral("qp7: the loudness window is the Mac's size"),
            win.w->width() == 780 && win.w->height() == 400 && win.w->minimumHeight() == 390);
    const bool wasOn = state(d)->loudness_enabled;
    const int wasMask = state(d)->loudness_output_mask;
    const float wasRef = state(d)->loudness_ref_spl, wasIntensity = state(d)->loudness_intensity;

    win.click(QStringLiteral("LoudnessWindow.header.switch"));
    d.check(QStringLiteral("qp7: the loudness switch writes"), waitState(d, [&](const DspState &s) { return s.loudness_enabled != wasOn; }));
    if (wasOn) {
        win.click(QStringLiteral("LoudnessWindow.header.switch"));
        waitState(d, [](const DspState &s) { return s.loudness_enabled; });
    }
    QQuickItem *graph = win.item(QStringLiteral("LoudnessWindow.graph"));
    d.check(QStringLiteral("qp7: the curve shows with loudness on, with its badge"),
            graph && graph->property("active").toBool() && graph->property("badge").toString() == QLatin1String("Curve at -40dB"));
    QQuickItem *ref = win.item(QStringLiteral("LoudnessWindow.ref"));
    if (ref) QMetaObject::invokeMethod(ref, "commit", Q_ARG(double, 75.0));
    d.check(QStringLiteral("qp7: the reference level writes"), waitState(d, [](const DspState &s) { return near(s.loudness_ref_spl, 75); }));
    QQuickItem *intensity = win.item(QStringLiteral("LoudnessWindow.intensity"));
    if (intensity) QMetaObject::invokeMethod(intensity, "live", Q_ARG(double, 150.0));
    d.check(QStringLiteral("qp7: the intensity writes"), waitState(d, [](const DspState &s) { return near(s.loudness_intensity, 150); }));
    d.check(QStringLiteral("qp7: the outputs show on firmware with the mask"),
            fx->loudnessMaskSupported() && win.shows(QStringLiteral("LoudnessWindow.outputs")) &&
                win.item(QStringLiteral("LoudnessWindow.output.") + QString::number(fx->numOutputs() - 1)));
    win.trigger(QStringLiteral("LoudnessWindow.outputs.first"));
    d.check(QStringLiteral("qp7: Presets > Slot 1 only is outputs 1 and 2"),
            waitState(d, [](const DspState &s) { return s.loudness_output_mask == 0x0003; }));
    win.click(QStringLiteral("LoudnessWindow.output.4"));
    d.check(QStringLiteral("qp7: an output chip toggles its output"),
            waitState(d, [](const DspState &s) { return s.loudness_output_mask == 0x0013; }));
    QQuickItem *chip = win.item(QStringLiteral("LoudnessWindow.output.4"));
    d.check(QStringLiteral("qp7: the chip's tooltip names the output"),
            chip && chip->property("toolTip").toString() == fx->outputName(4) && !fx->outputName(4).isEmpty());
    d.check(QStringLiteral("qp7: screenshot of loudness"), d.grab(win.w, QStringLiteral("window-loudness.png")));

    fx->setLoudnessMask(wasMask);
    fx->setLoudnessRef(wasRef);
    fx->setLoudnessIntensity(wasIntensity);
    fx->setEnabled(QStringLiteral("loudness"), wasOn);
    waitState(d, [&](const DspState &s) { return s.loudness_output_mask == wasMask && s.loudness_enabled == wasOn; });
    win.w->close();
}

// ---- Leveller ----

void leveller(SmokeDriver &d) {
    Fx *fx = Registrar::instance<Fx>();
    Win win = open(d, "tools.leveller", "LevellerWindow");
    if (!win.w || !state(d)) return;
    d.check(QStringLiteral("qp7: the leveller window is the Mac's size"),
            win.w->width() == 380 && win.w->height() == 540 && win.w->minimumHeight() == 400);
    const DspState s0 = *state(d);

    win.click(QStringLiteral("LevellerWindow.header.switch"));
    d.check(QStringLiteral("qp7: the leveller switch writes"),
            waitState(d, [&](const DspState &s) { return s.leveller_enabled != s0.leveller_enabled; }));
    d.check(QStringLiteral("qp7: the leveller's channels show with more than two inputs"),
            fx->levellerChannels() > 2 && fx->levellerMasksShown() == win.shows(QStringLiteral("LevellerWindow.channels")));
    if (fx->levellerMasksShown()) {
        win.trigger(QStringLiteral("LevellerWindow.channels.center"));
        d.check(QStringLiteral("qp7: Center only is channel 3 for both masks"),
                waitState(d, [](const DspState &s) { return s.leveller_detector_mask == 0x04 && s.leveller_apply_mask == 0x04; }));
        win.click(QStringLiteral("LevellerWindow.detector.0"));
        d.check(QStringLiteral("qp7: a detector chip toggles the detector only"),
                waitState(d, [](const DspState &s) { return s.leveller_detector_mask == 0x05 && s.leveller_apply_mask == 0x04; }));
        win.click(QStringLiteral("LevellerWindow.apply.1"));
        d.check(QStringLiteral("qp7: an apply chip toggles the apply mask only"),
                waitState(d, [](const DspState &s) { return s.leveller_detector_mask == 0x05 && s.leveller_apply_mask == 0x06; }));
    }
    QQuickItem *speedText = win.item(QStringLiteral("LevellerWindow.speedText"));
    win.click(QStringLiteral("LevellerWindow.speed.2"));
    d.check(QStringLiteral("qp7: the speed picker writes"), waitState(d, [](const DspState &s) { return s.leveller_speed == 2; }));
    d.check(QStringLiteral("qp7: the speed's description follows"),
            speedText && speedText->property("text").toString().startsWith(QLatin1String("Fast")));
    QQuickItem *gain = win.item(QStringLiteral("LevellerWindow.maxGain"));
    if (gain) QMetaObject::invokeMethod(gain, "commit", Q_ARG(double, 99.0));
    d.check(QStringLiteral("qp7: the largest gain is held to 35 dB"),
            waitState(d, [](const DspState &s) { return near(s.leveller_max_gain_db, 35); }));
    QQuickItem *gate = win.item(QStringLiteral("LevellerWindow.gate"));
    if (gate) QMetaObject::invokeMethod(gate, "commit", Q_ARG(double, -40.0));
    d.check(QStringLiteral("qp7: the gate writes"), waitState(d, [](const DspState &s) { return near(s.leveller_gate_db, -40); }));
    QQuickItem *amount = win.item(QStringLiteral("LevellerWindow.amount"));
    if (amount) QMetaObject::invokeMethod(amount, "commit", Q_ARG(double, 70.0));
    d.check(QStringLiteral("qp7: the amount writes"), waitState(d, [](const DspState &s) { return near(s.leveller_amount, 70); }));
    win.click(QStringLiteral("LevellerWindow.lookahead"));
    d.check(QStringLiteral("qp7: the lookahead switch writes"),
            waitState(d, [&](const DspState &s) { return s.leveller_lookahead != s0.leveller_lookahead; }));
    d.check(QStringLiteral("qp7: screenshot of the leveller"), d.grab(win.w, QStringLiteral("window-leveller.png")));

    fx->setLevellerMasks(s0.leveller_detector_mask, s0.leveller_apply_mask);
    fx->setLevellerSpeed(s0.leveller_speed);
    fx->setLevellerMaxGain(s0.leveller_max_gain_db);
    fx->setLevellerGate(s0.leveller_gate_db);
    fx->setLevellerAmount(s0.leveller_amount);
    fx->setLevellerLookahead(s0.leveller_lookahead);
    fx->setEnabled(QStringLiteral("leveller"), s0.leveller_enabled);
    waitState(d, [&](const DspState &s) { return s.leveller_enabled == s0.leveller_enabled; });
    win.w->close();
}

// ---- Psychoacoustic bass ----

void psybass(SmokeDriver &d) {
    Fx *fx = Registrar::instance<Fx>();
    Win win = open(d, "tools.psybass", "PsybassWindow");
    if (!win.w || !state(d)) return;
    d.check(QStringLiteral("qp7: the psybass window is the Mac's size"),
            win.w->width() == 780 && win.w->height() == 500 && win.w->minimumHeight() == 496);
    const DspState s0 = *state(d);
    d.check(QStringLiteral("qp7: the demo has psychoacoustic bass"),
            fx->psybassSupported() && win.shows(QStringLiteral("PsybassWindow.body")) &&
                !win.shows(QStringLiteral("PsybassWindow.unsupported")));

    if (!s0.psybass_enabled) {
        win.click(QStringLiteral("PsybassWindow.header.switch"));
        d.check(QStringLiteral("qp7: the psybass switch writes"), waitState(d, [](const DspState &s) { return s.psybass_enabled; }));
    }
    win.trigger(QStringLiteral("PsybassWindow.startingPoint.2"));
    d.check(QStringLiteral("qp7: a starting point writes its five values"), waitState(d, [](const DspState &s) {
        return near(s.psybass_cutoff_hz, 180) && near(s.psybass_harmonics_db, 6) && near(s.psybass_drive_db, 12) &&
               near(s.psybass_character_pct, 50) && near(s.psybass_original_db, -24);
    }));
    QQuickItem *cutoff = win.item(QStringLiteral("PsybassWindow.cutoff"));
    if (cutoff) QMetaObject::invokeMethod(cutoff, "commit", Q_ARG(double, 10.0));
    d.check(QStringLiteral("qp7: the cutoff is held to 30 Hz"), waitState(d, [](const DspState &s) { return near(s.psybass_cutoff_hz, 30); }));
    if (cutoff) QMetaObject::invokeMethod(cutoff, "commit", Q_ARG(double, 120.0));
    waitState(d, [](const DspState &s) { return near(s.psybass_cutoff_hz, 120); });
    QQuickItem *character = win.item(QStringLiteral("PsybassWindow.character"));
    if (character) QMetaObject::invokeMethod(character, "live", Q_ARG(double, 80.0));
    d.check(QStringLiteral("qp7: the character writes"), waitState(d, [](const DspState &s) { return near(s.psybass_character_pct, 80); }));
    QQuickItem *harm = win.item(QStringLiteral("PsybassWindow.graph.harmonics"));
    QQuickItem *orig = win.item(QStringLiteral("PsybassWindow.graph.original"));
    d.check(QStringLiteral("qp7: the sketch shows both bands"),
            harm && orig && harm->isVisible() && harm->width() > 0 && harm->height() > 0 && orig->width() > 0 &&
                harm->x() >= orig->width() - 1);
    win.trigger(QStringLiteral("PsybassWindow.outputs.noSub"));
    d.check(QStringLiteral("qp7: Exclude sub leaves the PDM output out"), waitState(d, [&](const DspState &s) {
        return s.psybass_output_mask == fx->outputsWithoutSubMask() && (s.psybass_output_mask & (1 << fx->pdmOutput())) == 0;
    }));
    win.click(QStringLiteral("PsybassWindow.output.0"));
    d.check(QStringLiteral("qp7: a psybass chip toggles its output"),
            waitState(d, [&](const DspState &s) { return (s.psybass_output_mask & 1) == 0; }));
    d.check(QStringLiteral("qp7: screenshot of psychoacoustic bass"), d.grab(win.w, QStringLiteral("window-psybass.png")));

    fx->setPsybassMask(s0.psybass_output_mask);
    fx->setPsybass(QStringLiteral("cutoff"), s0.psybass_cutoff_hz);
    fx->setPsybass(QStringLiteral("harmonics"), s0.psybass_harmonics_db);
    fx->setPsybass(QStringLiteral("drive"), s0.psybass_drive_db);
    fx->setPsybass(QStringLiteral("character"), s0.psybass_character_pct);
    fx->setPsybass(QStringLiteral("original"), s0.psybass_original_db);
    fx->setEnabled(QStringLiteral("psybass"), s0.psybass_enabled);
    waitState(d, [&](const DspState &s) { return s.psybass_enabled == s0.psybass_enabled; });
    win.w->close();
}

// ---- Read-only firmware (behaviour.md 14): shown, nothing writes ----

void readOnly(SmokeDriver &d) {
    Fx *fx = Registrar::instance<Fx>();
    DeviceModel *device = d.app().device;
    d.app().core->worker()->post([](FfiCore *c) { dspi_demo_open_wire(c, 17); });
    device->refresh();
    d.check(QStringLiteral("qp7: the V17 recording opens read-only"), d.waitFor(5000, [&]() { return device->readOnly(); }));
    d.waitIdle();
    d.check(QStringLiteral("qp7: V17 has none of the masks and no psychoacoustic bass"),
            !fx->writable() && !fx->crossfeedMaskSupported() && !fx->loudnessMaskSupported() && !fx->levellerMasksShown() &&
                !fx->psybassSupported());

    Win cf = open(d, "tools.crossfeed", "CrossfeedWindow");
    Win ld = open(d, "tools.loudness", "LoudnessWindow");
    Win pb = open(d, "tools.psybass", "PsybassWindow");
    if (!cf.w || !ld.w || !pb.w) return;
    const auto disabled = [](QQuickItem *it) { return it && !it->isEnabled(); };
    d.check(QStringLiteral("qp7: read-only disables the crossfeed window's writes"),
            disabled(cf.item(QStringLiteral("CrossfeedWindow.header.switch"))) &&
                disabled(cf.item(QStringLiteral("CrossfeedWindow.preset.0"))) &&
                disabled(cf.item(QStringLiteral("CrossfeedWindow.freq.slider"))) &&
                disabled(cf.item(QStringLiteral("CrossfeedWindow.itd"))));
    d.check(QStringLiteral("qp7: without the pair mask the pairs show only with a pair map"),
            cf.shows(QStringLiteral("CrossfeedWindow.pairs")) == Registrar::instance<ExtController>()->hasPairMap());
    d.check(QStringLiteral("qp7: firmware without the loudness mask has no outputs"), !ld.shows(QStringLiteral("LoudnessWindow.outputs")));
    d.check(QStringLiteral("qp7: firmware before V23 gets the psybass note"),
            pb.shows(QStringLiteral("PsybassWindow.unsupported")) && !pb.shows(QStringLiteral("PsybassWindow.body")));
    const int before = device->settersRun();
    cf.click(QStringLiteral("CrossfeedWindow.header.switch"));
    cf.click(QStringLiteral("CrossfeedWindow.preset.1"));
    ld.click(QStringLiteral("LoudnessWindow.header.switch"));
    fx->setCrossfeedFreq(1000);
    fx->setLoudnessRef(60);
    fx->setLevellerSpeed(1);
    d.waitIdle();
    d.check(QStringLiteral("qp7: read-only sends nothing"), device->settersRun() == before);
    d.check(QStringLiteral("qp7: screenshot of a read-only crossfeed window"), d.grab(cf.w, QStringLiteral("window-crossfeed-readonly.png")));
    d.check(QStringLiteral("qp7: screenshot of the psybass note"), d.grab(pb.w, QStringLiteral("window-psybass-unsupported.png")));
    cf.w->close();
    ld.w->close();
    pb.w->close();

    device->openDemo();
    d.check(QStringLiteral("qp7: the demo reopens writable"),
            d.waitFor(5000, [&]() { return !device->readOnly() && device->connected(); }));
    d.waitIdle();
}

void effectsSection(SmokeDriver &d) {
    d.check(QStringLiteral("qp7: the controllers are made"),
            Registrar::instance<Fx>() != nullptr && Registrar::instance<ExtController>() != nullptr);
    for (const char *id : { "tools.crossfeed", "tools.loudness", "tools.leveller", "tools.psybass" })
        d.check(QStringLiteral("qp7: the menu row %1 opens a window").arg(QLatin1String(id)),
                d.app().shell->handles(QString::fromLatin1(id)));
    maths(d);
    crossfeed(d);
    loudness(d);
    leveller(d);
    psybass(d);
    readOnly(d);
    d.mainWindow()->requestActivate();
    d.spin(100);
}

const SmokeDriver::Section effects("qp7-effects", 70, &effectsSection);

} // namespace
