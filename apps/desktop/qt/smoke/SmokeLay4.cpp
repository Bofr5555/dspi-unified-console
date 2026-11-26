// -- WP LAY4 -- Modern's Equaliser on the demo device: editing on the graph (a
// double-click adds a band, a drag and the wheel change it, the label follows),
// Tab through the band table with every field's text selected, the inspector
// (band, crossover, limiter, Linkwitz Transform with its draft), folded away
// under 1280 px, the Crossover and Delay & Gain pages, the Clear button at
// 1000x660, the band bar's option and the grey tones. Pictures of every page
// go to gallery/lay4-*.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "GraphController.h"
#include "HistoryController.h"
#include "LayoutController.h"
#include "PeqEditorController.h"
#include "PrefsMap.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"
#include "SpeakersController.h"
#include "render/PeqOverlay.h"

#include <QCoreApplication>
#include <QDir>
#include <QGuiApplication>
#include <QPointer>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <QWheelEvent>
#include <cmath>

namespace {

bool near(double a, double b, double tol = 1e-3) { return std::abs(a - b) <= tol; }

struct Eq {
    SmokeDriver &d;
    Shell *shell;
    LayoutController *layout;
    ChannelController *cc;
    SidebarController *sc;
    GraphController *g;
    QQuickWindow *main;
    int out = 0;   // the first output's wire channel

    void check(const char *what, bool ok) const { d.check(QStringLiteral("lay4: ") + QString::fromUtf8(what), ok); }
    const DspState *state() const { return d.app().device->state(); }
    FilterParams band(int b) const { return state() ? state()->filters[out][b] : FilterParams{}; }
    FilterParams xo(int ch, int b) const { return state() ? state()->xover[ch][b] : FilterParams{}; }
    QQuickItem *host() const { return qobject_cast<QQuickItem *>(shell->pageHost()); }
    void go(const char *tab) const {
        QVariant ok;
        QMetaObject::invokeMethod(host(), "go", Q_RETURN_ARG(QVariant, ok), Q_ARG(QVariant, QStringLiteral("equaliser")),
                                  Q_ARG(QVariant, QString::fromLatin1(tab)));
        settle();
    }
    void settle() const {
        d.spin(40);
        d.waitIdle();
        d.spin(80);
    }
    // The shown item named `name`: pages are kept, and one shows
    QQuickItem *shown(const QString &name, QQuickItem *from = nullptr) const {
        QQuickItem *at = from ? from : main->contentItem();
        if (!at->isVisible()) return nullptr;
        if (at->objectName() == name) return at;
        for (QQuickItem *c : at->childItems())
            if (QQuickItem *f = shown(name, c)) return f;
        return nullptr;
    }
    QQuickItem *shown(const char *name) const { return shown(QString::fromLatin1(name)); }
    static QPoint centre(QQuickItem *i) { return i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint(); }
    bool click(const QString &name, Qt::MouseButton button = Qt::LeftButton) const {
        QQuickItem *i = shown(name);
        if (!i || i->width() <= 0) return false;
        QTest::mouseClick(main, button, Qt::NoModifier, centre(i));
        settle();
        return true;
    }
    QString text(const char *name) const {
        QQuickItem *i = shown(name);
        return i ? i->property("text").toString() : QString();
    }
    // Types into a ChannelValueField and presses Enter
    bool type(const QString &name, const char *value) const {
        QQuickItem *f = shown(name + QStringLiteral(".field"));
        if (!f) return false;
        f->forceActiveFocus(Qt::TabFocusReason);
        d.spin(20);
        for (const char *c = value; *c; ++c) QTest::keyClick(main, *c);
        QTest::keyClick(main, Qt::Key_Return);
        settle();
        return true;
    }
    bool allSelected(QQuickItem *f) const {
        return f && f->hasActiveFocus() && !f->property("text").toString().isEmpty()
            && f->property("selectedText").toString() == f->property("text").toString();
    }
    void resize(int w, int h) const {
        main->resize(w, h);
        d.waitFor(1500, [&] { return main->contentItem()->width() == w && main->contentItem()->height() == h; });
        settle();
    }
    // `i` whole inside `outer`
    static bool inside(QQuickItem *i, QQuickItem *outer) {
        if (!i || !outer) return false;
        const QRectF r = i->mapRectToItem(outer, QRectF(0, 0, i->width(), i->height()));
        return r.left() >= -0.5 && r.top() >= -0.5 && r.right() <= outer->width() + 0.5 && r.bottom() <= outer->height() + 0.5;
    }
    PeqOverlay *overlay() const { return qobject_cast<PeqOverlay *>(shown("PeqEditor.overlay")); }
};

bool switchTo(Eq &e, const QString &layout, const char *shellName) {
    e.layout->setLayout(layout);
    QObject *loader = e.d.object(QStringLiteral("MainWindow.shell"));
    const bool ok = e.d.waitFor(3000, [&] {
        QObject *now = loader ? loader->property("item").value<QObject *>() : nullptr;
        return now && now->objectName() == QLatin1String(shellName);
    });
    QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    e.settle();
    return ok;
}

// The named popup under the main window's overlay
// (a page's popups are its items' children, not the window's)
QObject *popup(const Eq &e, const QString &name, QQuickItem *from = nullptr) {
    QQuickItem *at = from ? from : e.main->contentItem();
    if (QObject *o = at->findChild<QObject *>(name)) return o;
    for (QQuickItem *c : at->childItems())
        if (QObject *o = popup(e, name, c)) return o;
    return nullptr;
}
bool opened(const Eq &e, const char *name) {
    QObject *o = popup(e, QString::fromLatin1(name));
    return o && o->property("opened").toBool();
}

void answer(Eq &e, const QString &name, const char *how) {
    if (QObject *q = popup(e, name)) QMetaObject::invokeMethod(q, "finish", Q_ARG(QVariant, QString::fromLatin1(how)));
    e.settle();
}

void pagePart(Eq &e) {
    e.go("peq");
    for (const char *name : { "PageTabs.peq", "PageTabs.crossover", "PageTabs.delay", "EqualiserTools", "EqualiserPeq", "ChannelStrip",
                              "EqualiserPeq.channels", "GraphHost.plot", "EqualiserPeq.bands", "EqualiserPeq.inspector.sheet",
                              "EqualiserGraph.phase", "EqualiserGraph.all", "EqualiserGraph.popOut" })
        e.d.check(QStringLiteral("lay4: %1 shows").arg(QLatin1String(name)), e.shown(name) != nullptr);
    e.check("Classic's output card and band bar do not", !e.shown("OutputCard") && !e.shown("PeqBar"));
    e.sc->select(e.out);
    e.settle();
    QQuickItem *strip = e.shown("ChannelStrip");
    for (const char *name : { "ChannelStrip.enable", "ChannelStrip.route.0", "ChannelStrip.routeGain.1.field", "ChannelStrip.gain.field",
                              "ChannelStrip.gainSlider", "ChannelStrip.delay.field", "ChannelStrip.mute", "ChannelStrip.polarity",
                              "ChannelStrip.limiter" }) {
        QQuickItem *i = e.shown(name);
        e.d.check(QStringLiteral("lay4: the strip has %1, inside it").arg(QLatin1String(name)), i && Eq::inside(i, strip));
    }
    // The strip edits the device
    e.check("the strip's gain", e.type(QStringLiteral("ChannelStrip.gain"), "-4.5") && near(e.state()->output_gain_db[0], -4.5, 0.01));
    e.check("the strip's delay", e.type(QStringLiteral("ChannelStrip.delay"), "1.25") && near(e.state()->output_delay_ms[0], 1.25, 0.03));
    e.check("the strip's mute", e.click(QStringLiteral("ChannelStrip.mute")) && e.state()->output_muted[0]);
    e.click(QStringLiteral("ChannelStrip.mute"));
    e.cc->setOutputGain(0, 0);
    e.cc->setOutputDelay(0, 0);
    e.settle();
    // All curves, and back to the page's own
    e.click(QStringLiteral("EqualiserGraph.all"));
    e.check("All Curves shows every channel's", e.g->visibleChannels().size() == e.d.app().device->channels()->property("count").toInt()
                                                    || e.g->visibleChannels().size() > 2);
    e.click(QStringLiteral("EqualiserGraph.all"));
    e.check("and again the page's alone", e.g->visibleChannels() == QList<int>{ e.out });
    const bool phase = e.g->showPhase();
    e.click(QStringLiteral("EqualiserGraph.phase"));
    e.check("Phase switches the trace", e.g->showPhase() != phase);
    e.g->setShowPhase(phase);
}

void graphPart(Eq &e) {
    e.cc->clearBands(e.out, false);
    e.settle();
    PeqOverlay *ov = e.overlay();
    e.check("the graph edits the output's bands", ov && e.d.waitFor(3000, [&] { return ov->editedChannel() == e.out; }));
    if (!ov) return;
    ov->settle();
    e.check("an empty graph says how to begin", e.shown("PeqEditor.hint") != nullptr);
    e.check("and the inspector that no band is selected", e.shown("BandInspector.none") != nullptr);
    // A double-click on empty graph, away from the curve
    const QPoint at = ov->mapToScene(QPointF(ov->width() * 0.5, ov->height() * 0.3)).toPoint();
    QTest::mouseMove(e.main, at);
    e.d.spin(30);
    QTest::mouseDClick(e.main, Qt::LeftButton, Qt::NoModifier, at);
    e.settle();
    ov->settle();
    const FilterParams made = e.band(0);
    e.check("a double-click adds a bell there", made.filter_type == FILTER_PEAKING && made.gain > 1 && made.freq > 100);
    e.check("selected", Registrar::instance<PeqEditorController>()->selected() == QList<int>{ 0 });
    QQuickItem *label = e.shown("PeqEditor.bandLabel");
    e.check("the label on the graph has its values", label && label->property("shownName").toString().startsWith(QString::fromUtf8("1 · Bell"))
                                                         && label->property("shownName").toString().contains(QLatin1String("dB")));
    e.check("near its dot, inside the graph", label && Eq::inside(label, ov)
            && std::abs(label->x() + label->width() / 2 - ov->dotPoint(0).x()) < label->width()
            && std::abs(label->y() - ov->dotPoint(0).y()) < 60);
    e.check("the hint is gone", !e.shown("PeqEditor.hint"));
    e.check("the inspector shows the band", e.text("BandInspector.freq.field") == QString::number(double(made.freq), 'f', 1)
                                                || near(e.text("BandInspector.freq.field").toDouble(), made.freq, 0.1));
    e.d.grab(e.main, QStringLiteral("lay4-band.png"));

    // A drag of the dot, as a person does it
    const QString before = label ? label->property("shownName").toString() : QString();
    const QPoint from = ov->mapToScene(ov->dotPoint(0)).toPoint();
    QTest::mouseMove(e.main, from + QPoint(-30, 20));
    e.d.spin(20);
    QTest::mouseMove(e.main, from);
    e.d.spin(40);
    QTest::mousePress(e.main, Qt::LeftButton, Qt::NoModifier, from);
    e.d.spin(20);
    const QPoint to = from + QPoint(70, 30);
    for (int i = 1; i <= 12; ++i) {
        QTest::mouseMove(e.main, from + (to - from) * i / 12);
        e.d.spin(16);
    }
    QString during;
    if (QQuickItem *l = e.shown("PeqEditor.bandLabel")) during = l->property("shownName").toString();
    const QString field = e.text("BandInspector.freq.field");
    e.check("the label and the inspector follow the drag", !during.isEmpty() && during != before && field.toDouble() > made.freq * 1.05);
    QTest::mouseRelease(e.main, Qt::LeftButton, Qt::NoModifier, to);
    e.settle();
    ov->settle();
    const FilterParams dragged = e.band(0);
    e.check("a drag raises the frequency and lowers the gain", dragged.freq > made.freq * 1.05 && dragged.gain < made.gain - 0.5);

    // The wheel over it: Q
    const QPoint dot = ov->mapToScene(ov->dotPoint(0)).toPoint();
    QTest::mouseMove(e.main, dot);
    e.d.spin(20);
    QWheelEvent wheel(QPointF(dot), e.main->mapToGlobal(QPointF(dot)), QPoint(), QPoint(0, 120), Qt::NoButton, Qt::NoModifier, Qt::NoScrollPhase, false);
    QCoreApplication::sendEvent(e.main, &wheel);
    e.d.spin(600);
    e.settle();
    e.check("the wheel over it changes its Q", std::abs(e.band(0).q - dragged.q) > 0.01);

    // Enter on the graph types the band's values in the inspector
    ov->forceActiveFocus();
    QTest::keyClick(e.main, Qt::Key_Return);
    e.settle();
    QQuickItem *freq = e.shown("BandInspector.freq.field");
    e.check("Enter on the graph types in the inspector, all selected", e.allSelected(freq));
    QTest::keyClick(e.main, Qt::Key_Escape);
}

void tablePart(Eq &e) {
    e.cc->clearBands(e.out, false);
    for (int b : { 0, 1 }) {
        e.cc->setBandType(e.out, b, false, FILTER_PEAKING);
        e.cc->setBandValue(e.out, b, false, QStringLiteral("freq"), b ? 3000 : 300);
        e.cc->setBandValue(e.out, b, false, QStringLiteral("gain"), b ? -3 : 3);
    }
    e.settle();
    auto *pc = Registrar::instance<PeqEditorController>();
    pc->forget();
    e.settle();
    const auto name = [](int b, const char *f) { return QStringLiteral("EqualiserPeq.bands.band.%1.%2.field").arg(b).arg(QLatin1String(f)); };
    QPointer<QQuickItem> freq = e.shown(name(0, "freq"));
    e.check("the table's first frequency", !freq.isNull());
    if (!freq) return;
    QTest::mouseClick(e.main, Qt::LeftButton, Qt::NoModifier, Eq::centre(freq));
    e.d.spin(60);
    e.check("a click into a field selects all its text", e.allSelected(freq));
    e.check("and selects its band", pc->selected() == QList<int>{ 0 } && e.shown("BandInspector.freq.field")
                                        && near(e.text("BandInspector.freq.field").toDouble(), 300, 0.1));
    // Typed and tabbed on: committed, the next field all selected, the table not made anew
    for (const char *c = "440"; *c; ++c) QTest::keyClick(e.main, *c);
    QTest::keyClick(e.main, Qt::Key_Tab);
    e.d.spin(40);
    e.check("Tab takes the typed value and selects the next field's text", e.allSelected(e.shown(name(0, "gain"))));
    e.settle();
    e.check("the value reached the device", near(e.band(0).freq, 440, 0.5));
    e.check("the table kept its fields and the focus", freq && freq == e.shown(name(0, "freq")) && e.allSelected(e.shown(name(0, "gain"))));
    QTest::keyClick(e.main, Qt::Key_Tab);
    e.d.spin(40);
    e.check("Tab to Q", e.allSelected(e.shown(name(0, "q"))));
    // On to the next band's fields, whatever stands between
    int tabs = 0;
    while (tabs < 6 && !(e.shown(name(1, "freq")) && e.shown(name(1, "freq"))->hasActiveFocus())) {
        QTest::keyClick(e.main, Qt::Key_Tab);
        e.d.spin(30);
        ++tabs;
    }
    e.check("Tab goes on to the next band's frequency, all selected", e.allSelected(e.shown(name(1, "freq"))));
    e.d.spin(60);
    e.check("and the inspector and the graph follow", pc->selected() == QList<int>{ 1 } && near(e.text("BandInspector.freq.field").toDouble(), 3000, 0.1));
    QTest::keyClick(e.main, Qt::Key_Backtab);
    e.d.spin(30);
    e.check("Shift+Tab goes back", !e.shown(name(1, "freq"))->hasActiveFocus());
    // A row that is off: the inspector offers the shapes
    QQuickItem *off = e.shown(QStringLiteral("EqualiserPeq.bands.band.4.type"));
    if (off) off->forceActiveFocus(Qt::TabFocusReason);
    e.settle();
    e.check("a row that is off shows in the inspector", e.shown("BandInspector.off") && pc->selected().isEmpty());
    e.check("a shape there switches it on", e.click(QStringLiteral("BandInspector.shape.%1").arg(FILTER_LOWSHELF)) && e.band(4).filter_type == FILTER_LOWSHELF);
    e.check("and its dot is selected", e.d.waitFor(1000, [&] { return pc->selected() == QList<int>{ 4 }; }));
    e.click(QStringLiteral("BandInspector.slope.0"));
    e.check("the slope steps it to 6 dB/oct", e.band(4).filter_type == FILTER_LOWSHELF1);
    e.check("another shape keeps the gentler slope", e.click(QStringLiteral("BandInspector.shape.%1").arg(FILTER_HIGHSHELF)) && e.band(4).filter_type == FILTER_HIGHSHELF1);
    e.check("the inspector's frequency", e.type(QStringLiteral("BandInspector.freq"), "5000") && near(e.band(4).freq, 5000, 1));
    e.check("and gain", e.type(QStringLiteral("BandInspector.gain"), "2.5") && near(e.band(4).gain, 2.5, 0.01));
    e.click(QStringLiteral("BandInspector.invert"));
    e.check("Invert", near(e.band(4).gain, -2.5, 0.01));
    e.click(QStringLiteral("BandInspector.on"));
    e.check("Band on", e.band(4).bypass);
    e.click(QStringLiteral("BandInspector.reset"));
    e.check("Reset switches it off and clears it", e.band(4).filter_type == FILTER_FLAT && near(e.band(4).gain, 0) && near(e.band(4).freq, 1000, 1));
    if (auto *hc = Registrar::instance<HistoryController>()) {
        hc->undo();
        e.settle();
        e.check("one undo brings it back", e.d.waitFor(2000, [&] { return e.band(4).filter_type == FILTER_HIGHSHELF1 && near(e.band(4).gain, -2.5, 0.01); }));
    }
    e.cc->clearBand(e.out, 4, false);
    e.settle();
}

void inspectorPart(Eq &e) {
    // The sections fold by their heading
    const auto unfold = [&](const char *id) {
        const QString heading = QStringLiteral("Inspector.%1.heading").arg(QLatin1String(id));
        QQuickItem *section = e.shown(QStringLiteral("Inspector.") + QLatin1String(id));
        if (section && !section->property("expanded").toBool())
            if (QQuickItem *h = e.shown(heading)) QMetaObject::invokeMethod(h, "click");
        e.settle();
        return section && section->property("expanded").toBool();
    };
    // Crossover
    e.check("the crossover section", unfold("crossover"));
    QQuickItem *hp = e.shown("Inspector.highPass.type");
    int index = -1;
    if (hp) {
        const QStringList choices = hp->property("model").toStringList();
        for (int i = 0; i < choices.size(); ++i)
            if (choices[i].startsWith(QLatin1String("Linkwitz-Riley")) && choices[i].contains(QLatin1String("24"))) index = i;
        QMetaObject::invokeMethod(hp, "activated", Q_ARG(int, index));
    }
    e.settle();
    e.check("a high pass chosen there is the device's", index > 0 && e.xo(e.out, 0).filter_type == ChannelController::crossoverType(1, 4, false));
    e.check("its frequency", e.type(QStringLiteral("Inspector.highPass.freq"), "2500") && near(e.xo(e.out, 0).freq, 2500, 2));
    QQuickItem *section = e.shown("Inspector.crossover");
    e.check("the heading sums it up", section && section->property("summary").toString().contains(QLatin1String("2.5")));
    QQuickItem *lp = e.shown("Inspector.lowPass.type");
    if (lp) QMetaObject::invokeMethod(lp, "activated", Q_ARG(int, 1));
    e.settle();
    e.check("a low pass takes the next band", e.xo(e.out, 1).filter_type != FILTER_FLAT && e.cc->isLowPass(e.xo(e.out, 1).filter_type));
    if (lp) QMetaObject::invokeMethod(lp, "activated", Q_ARG(int, 0));
    e.settle();
    e.check("Off switches it off again", e.xo(e.out, 1).filter_type == FILTER_FLAT);
    // The user's speaker layout has a tweeter there (the demo's layout is kept as it was)
    if (auto *speakers = Registrar::instance<SpeakersController>()) {
        e.d.waitFor(3000, [&] { return speakers->loaded(); });
        const QString was = speakers->roleOf(0);
        speakers->writeBack(0, QStringLiteral("high"));
        e.d.waitFor(3000, [&] { return speakers->roleOf(0) == QLatin1String("high") && speakers->status() != QLatin1String("none"); });
        e.settle();
        e.check("a tweeter with its high pass is not warned about", speakers->roleOf(0) == QLatin1String("high") && !e.shown("Inspector.tweeter"));
        e.cc->clearBands(e.out, true);
        e.settle();
        e.check("one without it is", e.shown("Inspector.tweeter") != nullptr);
        speakers->writeBack(0, was.isEmpty() ? QStringLiteral("full") : was);
        e.d.waitFor(3000, [&] { return speakers->roleOf(0) != QLatin1String("high"); });
        e.settle();
        e.check("and no other", !e.shown("Inspector.tweeter"));
    }
    e.cc->clearBands(e.out, true);

    // Limiter
    e.check("the limiter section", unfold("limiter"));
    const bool limited = e.state()->limiter_enabled[0];
    QQuickItem *toggle = e.shown("LimiterSettings.enabled");
    if (toggle) QMetaObject::invokeMethod(toggle, "click");
    if (toggle) QMetaObject::invokeMethod(toggle, "toggled");
    e.settle();
    e.check("the limiter's switch is the device's", toggle && e.state()->limiter_enabled[0] != limited);
    e.cc->setLimiterEnabled(0, limited);
    e.settle();
    e.check("no second heading in it", !e.shown("LimiterSettings.heading"));
    e.cc->requestLimiterSettings(0);
    e.settle();
    e.check("the limiter's settings asked for elsewhere are the inspector's, in no popover",
            e.shown("LimiterSettings.threshold") && !opened(e, "LimiterSettings"));
    e.d.grab(e.main, QStringLiteral("lay4-inspector-limiter.png"));
    QMetaObject::invokeMethod(e.shown("Inspector.limiter.heading"), "click");
    e.settle();

    // Linkwitz
    e.check("the Linkwitz section", unfold("linkwitz"));
    QQuickItem *add = e.shown("Inspector.addLinkwitz");
    e.check("an output without one offers a band", add && add->isEnabled());
    if (add) QMetaObject::invokeMethod(add, "click");
    e.settle();
    int lt = -1;
    for (int b = 0; b < 10; ++b)
        if (e.band(b).filter_type == FILTER_LINKWITZ_TRANSFORM) lt = b;
    e.check("it takes it, flat", lt >= 0 && near(e.band(lt).freq, e.band(lt).gain, 0.1));
    const auto draft = [&](const char *value) {
        QQuickItem *f = e.shown("Linkwitz.fp.field");
        if (!f) return false;
        f->setProperty("text", QString::fromLatin1(value));
        QMetaObject::invokeMethod(f, "textEdited"); // -- WP INTE -- (REVE C1) as typed: Enter writes only typed text
        QMetaObject::invokeMethod(f, "accepted");
        e.settle();
        return true;
    };
    e.check("its draft is not sent", draft("500") && lt >= 0 && !near(e.band(lt).gain, 500, 1) && e.text("Linkwitz.state").contains(QLatin1String("Not")));
    // Another output and back: the draft was this one's
    e.sc->select(e.out + 1);
    e.settle();
    e.sc->select(e.out);
    e.settle();
    e.check("another channel drops the draft", e.text("Linkwitz.state") == QLatin1String("Applied"));
    e.check("Apply sends it", draft("500") && e.shown("Linkwitz.apply"));
    if (QQuickItem *apply = e.shown("Linkwitz.apply")) QMetaObject::invokeMethod(apply, "click");
    e.settle();
    e.check("and then the band has it", lt >= 0 && near(e.band(lt).gain, 500, 1) && e.text("Linkwitz.state") == QLatin1String("Applied"));
    e.d.grab(e.main, QStringLiteral("lay4-inspector-linkwitz.png"));
    // A draft is not carried to another output's own transform
    e.cc->setBandType(e.out + 1, lt, false, FILTER_LINKWITZ_TRANSFORM);
    e.settle();
    draft("700");
    e.sc->select(e.out + 1);
    e.settle();
    e.check("nor to another output's transform", e.text("Linkwitz.state") == QLatin1String("Applied"));
    e.cc->clearBand(e.out + 1, lt, false);
    e.sc->select(e.out);
    e.cc->clearBands(e.out, false);
    e.settle();
    if (QQuickItem *h = e.shown("Inspector.linkwitz.heading")) QMetaObject::invokeMethod(h, "click");

    // An input: what it feeds
    e.sc->select(0);
    e.settle();
    e.check("an input's feeds", unfold("feeds") && e.shown("Inspector.editRouting"));
    QQuickItem *fed = nullptr;
    for (int o = 0; o < e.cc->numOutputs() && !fed; ++o) fed = e.shown(QStringLiteral("Inspector.feeds.%1").arg(o));
    e.check("list the outputs it is connected to", fed != nullptr);
    e.check("an input has no crossover, limiter or Linkwitz there",
            !e.shown("Inspector.crossover") && !e.shown("Inspector.limiter") && !e.shown("Inspector.linkwitz"));
    e.d.grab(e.main, QStringLiteral("lay4-input.png"));
    if (fed) QMetaObject::invokeMethod(fed, "click");
    e.settle();
    e.check("a feed goes to its output", e.sc->selectedChannel() >= e.out);
    e.sc->select(e.out);
    e.settle();
}

void foldPart(Eq &e) {
    const auto sheet = [&] { return e.shown("EqualiserPeq.inspector.sheet"); };
    const auto pane = [&] { return e.d.item(QStringLiteral("EqualiserPeq.inspector")); };
    e.resize(1280, 800);
    e.check("at 1280 px the inspector is beside the page", sheet() && pane()->width() >= 300);
    e.resize(1279, 800);
    e.check("below it is folded away", !sheet() && pane()->width() == 0);
    const qreal graph = e.shown("GraphHost.plot") ? e.shown("GraphHost.plot")->width() : 0;
    e.click(QStringLiteral("EqualiserTools.inspector"));
    e.check("its button opens it over the page", sheet() && pane()->width() == 0 && e.shown("GraphHost.plot")->width() == graph);
    e.check("inside the window", sheet() && Eq::inside(sheet(), e.main->contentItem()));
    e.d.grab(e.main, QStringLiteral("lay4-inspector-over.png"));
    e.click(QStringLiteral("EqualiserPeq.inspector.close"));
    e.check("its close button folds it again", !sheet());
    // Enter on the graph needs it
    e.cc->setBandType(e.out, 0, false, FILTER_PEAKING);
    e.settle();
    Registrar::instance<PeqEditorController>()->rowClicked(0, 0);
    if (PeqOverlay *ov = e.overlay()) ov->forceActiveFocus();
    QTest::keyClick(e.main, Qt::Key_Return);
    e.settle();
    e.check("typing a band's values opens it", sheet() && e.allSelected(e.shown("BandInspector.freq.field")));
    e.resize(1280, 800);
    e.check("wide again it is beside the page", sheet() && pane()->width() >= 300);
    e.click(QStringLiteral("EqualiserTools.inspector"));
    e.check("hidden by its button, it stays so", !sheet() && e.layout->arrangement()->value(QStringLiteral("inspector")).toString() == QLatin1String("hidden"));
    e.click(QStringLiteral("EqualiserTools.inspector"));
    e.check("and shows again", sheet() != nullptr);

    // The band bar, as an option
    e.layout->arrangement()->setProperty("bandBar", QStringLiteral("shown"));
    e.settle();
    e.check("the band bar docks under the graph where it is wanted", e.shown("PeqBar") && Eq::inside(e.shown("EqualiserPeq.bands"), e.host()));
    e.d.grab(e.main, QStringLiteral("lay4-band-bar.png"));
    e.layout->arrangement()->setProperty("bandBar", QStringLiteral("hidden"));
    e.settle();
    e.check("and goes", !e.shown("PeqBar"));
}

// -- WP DL8 -- The Crossover page's graph in a GraphFrame, and its four
// bands only where the table cannot show them (DESREV §5)
void dl8CrossoverPart(Eq &e) {
    QQuickItem *labels = e.shown("EqualiserCrossover.plot.labels");
    bool inside = labels && labels->width() > 100;
    int texts = 0;
    if (inside)
        for (QQuickItem *c : labels->childItems()) {
            if (!c->isVisible() || !c->property("text").isValid()) continue;
            ++texts;
            inside = inside && Eq::inside(c, labels);
        }
    e.d.check(QStringLiteral("dl8: the crossover graph's %1 axis labels are whole inside it").arg(texts), inside && texts >= 10);
    QQuickItem *grid = e.shown("EqualiserCrossover.grid");
    e.d.check(QStringLiteral("dl8: its grid is BodePlot's, the frame draws none"), grid && e.shown("EqualiserCrossover.plot")
                  && !e.shown("EqualiserCrossover.plot")->property("grid").toBool());
    e.cc->clearBands(e.out + 1, true);
    e.cc->setBandType(e.out + 1, 0, true, ChannelController::crossoverType(1, 4, true));
    e.cc->setBandType(e.out + 1, 1, true, ChannelController::crossoverType(1, 4, false));
    e.settle();
    e.d.check(QStringLiteral("dl8: Linkwitz-Riley alone, one of each: the table says it all, no band list"),
              !e.shown("EqualiserCrossover.bands") && !e.shown("EqualiserCrossover.bandsHeading") && e.shown("EqualiserCrossover.legend"));
    e.cc->setBandType(e.out + 1, 1, true, ChannelController::crossoverType(3, 4, false));
    e.settle();
    e.d.check(QStringLiteral("dl8: a Bessel high pass brings the band list back"),
              e.shown("EqualiserCrossover.bands") && e.shown("EqualiserCrossover.bandsHeading"));
    e.cc->setBandType(e.out + 1, 1, true, ChannelController::crossoverType(1, 4, false));
    e.cc->setBandType(e.out + 1, 2, true, ChannelController::crossoverType(1, 2, false));
    e.settle();
    e.d.check(QStringLiteral("dl8: so does a second high pass (the table shows the first, +1)"),
              e.shown("EqualiserCrossover.bands") && e.text("EqualiserCrossover.row.1.highPass.more") == QLatin1String("+1"));
}

void crossoverPart(Eq &e) {
    e.sc->select(e.out);
    e.go("crossover");
    e.check("the Crossover page has every output in its table", e.shown("EqualiserCrossover.row.0") && e.cc->numOutputs() > 1
            && e.d.item(QStringLiteral("EqualiserCrossover.row.%1").arg(e.cc->numOutputs() - 1)));
    QQuickItem *curves = e.shown("EqualiserCrossover.curves");
    int enabled = 0;
    for (int o = 0; o < e.cc->numOutputs(); ++o) enabled += e.state()->output_enabled[o] ? 1 : 0;
    e.check("and every enabled output's curve", curves && e.d.waitFor(2000, [&] { return curves->property("drawnChannels").value<QList<int>>().size() == enabled; }));
    e.check("the graph's band editor is not on it", !e.overlay() && Registrar::instance<PeqEditorController>()->editedChannel() < 0);
    // The second output's low pass, in the table
    QQuickItem *lp = e.shown("EqualiserCrossover.row.1.lowPass.type");
    int index = -1;
    if (lp) {
        const QStringList choices = lp->property("model").toStringList();
        for (int i = 0; i < choices.size(); ++i)
            if (choices[i].startsWith(QLatin1String("Butterworth")) && choices[i].contains(QLatin1String("12 "))) index = i;
        QMetaObject::invokeMethod(lp, "activated", Q_ARG(int, index));
    }
    e.settle();
    e.check("a low pass chosen in the table is that output's", index > 0 && e.xo(e.out + 1, 0).filter_type == ChannelController::crossoverType(2, 2, true));
    e.check("and its frequency", e.type(QStringLiteral("EqualiserCrossover.row.1.lowPass.freq"), "180") && near(e.xo(e.out + 1, 0).freq, 180, 1));
    e.cc->setBandType(e.out + 1, 1, true, ChannelController::crossoverType(1, 2, true));
    e.settle();
    e.check("a second low pass counts", e.text("EqualiserCrossover.row.1.lowPass.more") == QLatin1String("+1"));
    QQuickItem *row = e.shown("EqualiserCrossover.row.1");
    if (row) QTest::mouseClick(e.main, Qt::LeftButton, Qt::NoModifier, row->mapToScene(QPointF(80, row->height() / 2)).toPoint());
    e.settle();
    // -- WP DL8 -- the four bands show because a Butterworth and a second low pass are there
    e.check("a row selects its output, and its four bands show", e.sc->selectedChannel() == e.out + 1 && e.shown("EqualiserCrossover.bands"));
    e.d.grab(e.main, QStringLiteral("lay4-crossover.png"));
    dl8CrossoverPart(e);
    e.cc->clearBands(e.out + 1, true);
    e.sc->select(0);
    e.settle();
    e.check("an input has none", !e.shown("EqualiserCrossover.bands"));
    e.sc->select(e.out);
    e.settle();
}

void delayPart(Eq &e) {
    e.go("delay");
    e.check("Delay & Gain has every output, and no channel list", e.shown("EqualiserDelay.row.0") && !e.shown("EqualiserDelay.channels") && !e.shown("ChannelStrip"));
    e.check("a delay", e.type(QStringLiteral("EqualiserDelay.row.0.delay"), "2") && near(e.state()->output_delay_ms[0], 2, 0.03));
    e.check("its distance", near(e.text("EqualiserDelay.row.0.distance.field").toDouble(), 68.6, 1));
    e.check("a distance sets the delay", e.type(QStringLiteral("EqualiserDelay.row.1.distance"), "34.3") && near(e.state()->output_delay_ms[1], 1, 0.03));
    QQuickItem *a = e.shown("EqualiserDelay.row.0.arrival"), *b = e.shown("EqualiserDelay.row.1.arrival");
    e.check("the latest has the longest bar", a && b && near(a->property("share").toDouble(), 1, 0.01) && near(b->property("share").toDouble(), 0.5, 0.03));
    // Enter, then on to another field: the first one's value stays as typed
    e.check("a value typed with Enter stays when the field is left", near(e.state()->output_delay_ms[0], 2, 0.03));
    e.check("a gain", e.type(QStringLiteral("EqualiserDelay.row.1.gain"), "-6") && near(e.state()->output_gain_db[1], -6, 0.01));

    // Polarity: every connected input, one undo step
    e.cc->setRoute(0, 0, true, 0, false);
    e.cc->setRoute(1, 0, true, -3, false);
    e.settle();
    e.check("the polarity reads Normal", e.text("EqualiserDelay.row.0.polarity") == QLatin1String("Normal"));
    e.click(QStringLiteral("EqualiserDelay.row.0.polarity"));
    e.check("inverted, every connected input is, its gain kept",
            e.state()->matrix_invert[0][0] && e.state()->matrix_invert[1][0] && near(e.state()->matrix_gain[1][0], -3, 0.01)
                && e.text("EqualiserDelay.row.0.polarity") == QLatin1String("Inverted"));
    if (auto *hc = Registrar::instance<HistoryController>()) {
        hc->undo();
        e.settle();
        e.check("one undo step takes both back", e.d.waitFor(2000, [&] { return !e.state()->matrix_invert[0][0] && !e.state()->matrix_invert[1][0]; }));
    }
    e.cc->setRoute(0, 0, true, 0, true);
    e.settle();
    e.check("some of them reads Mixed", e.text("EqualiserDelay.row.0.polarity") == QLatin1String("Mixed"));
    e.click(QStringLiteral("EqualiserDelay.row.0.polarity"));
    e.check("and goes to Normal", e.cc->outputPolarity(0) == 0);
    e.cc->setRoute(1, 0, false, 0, false);
    e.click(QStringLiteral("EqualiserDelay.row.1.mute"));
    e.check("mute", e.state()->output_muted[1]);
    e.cc->setOutputMute(1, false);
    const bool limited = e.state()->limiter_enabled[1];
    e.click(QStringLiteral("EqualiserDelay.row.1.limiter"));
    e.check("limiter", e.state()->limiter_enabled[1] != limited);
    e.cc->setLimiterEnabled(1, limited);
    e.settle();
    e.click(QStringLiteral("EqualiserDelay.row.1.limiter"), Qt::RightButton);
    e.check("its right-click shows its settings in the inspector", e.sc->selectedChannel() == e.out + 1 && e.shown("LimiterSettings.threshold"));
    if (QQuickItem *h = e.shown("Inspector.limiter.heading")) QMetaObject::invokeMethod(h, "click");
    e.settle();

    // An enable that switches the other side of Core 1 off asks, here too
    const int pdm = e.cc->pdmOutput();
    // -- WP UA1 -- (UXP C5) outputs 4 and the PDM one have no route: open the fold they are under
    if (!e.shown(QStringLiteral("EqualiserDelay.row.%1.enable").arg(pdm))) e.click(QStringLiteral("EqualiserDelay.unused"));
    e.check("an output's enable", e.click(QStringLiteral("EqualiserDelay.row.3.enable")) && e.state()->output_enabled[3]);
    e.check("the PDM output would take its core", e.cc->enableWouldConflict(pdm));
    QObject *question = popup(e, QStringLiteral("EqualiserDelay.enableQuestion"));
    e.click(QStringLiteral("EqualiserDelay.row.%1.enable").arg(pdm));
    e.check("so its enable asks first", question && e.d.waitFor(1000, [&] { return question->property("opened").toBool(); }));
    answer(e, QStringLiteral("EqualiserDelay.enableQuestion"), "reject");
    e.check("refused, it stays off", !e.state()->output_enabled[pdm] && e.state()->output_enabled[3]
                && e.shown(QStringLiteral("EqualiserDelay.row.%1.enable").arg(pdm)) && !e.shown(QStringLiteral("EqualiserDelay.row.%1.enable").arg(pdm))->property("checked").toBool());
    e.cc->requestOutputEnable(3, false);
    e.settle();
    e.d.grab(e.main, QStringLiteral("lay4-delay.png"));
    e.cc->setOutputDelay(0, 0);
    e.cc->setOutputDelay(1, 0);
    e.cc->setOutputGain(1, 0);
    e.sc->select(e.out);
    e.settle();
}

// Review RB: the Clear button at the smallest window
void clearPart(Eq &e) {
    e.go("peq");
    e.resize(1000, 660);
    for (const int ch : { 0, e.out }) {
        e.sc->select(ch);
        e.settle();
        QQuickItem *clear = e.shown("EqualiserPeq.bands.clearAll"), *list = e.shown("EqualiserPeq.bands"), *page = e.shown("EqualiserPeq");
        e.d.check(QStringLiteral("lay4: the Clear button at 1000x660 is inside its card and the page (channel %1)").arg(ch),
                  clear && Eq::inside(clear, list) && Eq::inside(list, page));
        QQuickItem *rows = e.shown("EqualiserPeq.bands.rows");
        e.d.check(QStringLiteral("lay4: the table shows four rows or more there (%1 px)").arg(rows ? rows->height() : 0), rows && rows->height() >= 4 * 28);
        e.d.check(QStringLiteral("lay4: and the graph keeps %1 px").arg(e.shown("GraphHost.plot") ? e.shown("GraphHost.plot")->height() : 0),
                  e.shown("GraphHost.plot") && e.shown("GraphHost.plot")->height() >= 120);
        e.d.check(QStringLiteral("lay4: the strip fits"), Eq::inside(e.shown("ChannelStrip"), page));
    }
    e.sc->select(0);
    if (!e.cc->isLinked(0)) e.cc->linkPair(0, 0);
    e.settle();
    e.cc->setBandType(0, 0, false, FILTER_PEAKING);
    e.cc->setBandValue(0, 0, false, QStringLiteral("gain"), 3);
    e.settle();
    e.check("a linked pair's Clear names both", e.text("EqualiserPeq.bands.clearAll").contains(QLatin1String("1/2")));
    e.click(QStringLiteral("EqualiserPeq.bands.clearAll"));
    e.check("it asks", e.d.waitFor(1500, [&] { return opened(e, "EqualiserPeq.bands.clearQuestion"); }));
    answer(e, QStringLiteral("EqualiserPeq.bands.clearQuestion"), "accept");
    e.check("and clears both", e.state()->filters[0][0].filter_type == FILTER_FLAT && e.state()->filters[1][0].filter_type == FILTER_FLAT);
    e.resize(1280, 800);
    e.sc->select(e.out);
    e.settle();
}

// -- WP DL8 -- The tab row's tools: Headphone Profile for an input only, the
// filter files under one "Files ▾"; and at 720 px high the band table keeps
// every band in use and one more (DESREV §5)
void dl8Part(Eq &e) {
    e.go("peq");
    e.sc->select(e.out);
    e.settle();
    e.d.check(QStringLiteral("dl8: no Headphone Profile for an output"), !e.shown("EqualiserTools.autoeq.browse") && e.shown("EqualiserTools.files"));
    e.d.check(QStringLiteral("dl8: no Import… / Export… buttons of their own"),
              !e.shown("EqualiserTools.file.importFilters") && !e.shown("EqualiserTools.file.exportFilters"));
    e.sc->select(0);
    e.settle();
    e.d.check(QStringLiteral("dl8: Headphone Profile for an input"), e.shown("EqualiserTools.autoeq.browse") != nullptr);
    e.click(QStringLiteral("EqualiserTools.files"));
    QObject *menu = popup(e, QStringLiteral("EqualiserTools.files.menu"));
    QObject *import = popup(e, QStringLiteral("EqualiserTools.files.menu.file.importFilters"));
    QObject *exportRow = popup(e, QStringLiteral("EqualiserTools.files.menu.file.exportFilters"));
    e.d.check(QStringLiteral("dl8: Files ▾ opens Import Filters… and Export Filters…"),
              menu && e.d.waitFor(1500, [&] { return menu->property("opened").toBool(); }) && import && exportRow
                  && import->property("text").toString() == QStringLiteral("Import Filters…")
                  && exportRow->property("text").toString() == QStringLiteral("Export Filters…"));
    if (menu) QMetaObject::invokeMethod(menu, "close");
    e.settle();

    e.resize(1100, 720);
    e.sc->select(e.out);
    e.cc->clearBands(e.out, false);
    for (int b = 0; b < 6; ++b) {
        e.cc->setBandType(e.out, b, false, FILTER_PEAKING);
        e.cc->setBandValue(e.out, b, false, QStringLiteral("freq"), 100 * (b + 1));
    }
    e.settle();
    QQuickItem *rows = e.shown("EqualiserPeq.bands.rows"), *plot = e.shown("GraphHost.plot");
    e.d.check(QStringLiteral("dl8: at 1100x720 the table shows the six bands in use and one more (%1 px)").arg(rows ? rows->height() : 0),
              rows && rows->height() >= 7 * 29 - 1);
    e.d.check(QStringLiteral("dl8: and the graph keeps %1 px").arg(plot ? plot->height() : 0), plot && plot->height() >= 160);
    e.d.grab(e.main, QStringLiteral("dl8-peq-720.png"));
    e.cc->clearBands(e.out, false);
    e.resize(1280, 800);
}

void tonePart(Eq &e) {
    const Qt::ColorScheme before = e.shell->colorSchemeOverride();
    e.shell->setColorSchemeOverride(Qt::ColorScheme::Dark);
    e.settle();
    const QColor grey = e.main->color();
    e.check("Modern is neutral grey", grey.isValid() && grey.red() == grey.green() && grey.green() == grey.blue());
    e.layout->arrangement()->setProperty("tone", QStringLiteral("darker"));
    e.settle();
    const QColor darker = e.main->color();
    e.check("Darker grey is darker, neutral too", darker.lightness() < grey.lightness() && darker.red() == darker.blue());
    QDir(e.d.outDir()).mkpath(QStringLiteral("gallery"));
    e.check("screenshot in Darker grey", e.d.grab(e.main, QStringLiteral("gallery/lay4-darker-dark.png")));
    e.shell->setColorSchemeOverride(Qt::ColorScheme::Light);
    e.settle();
    e.d.grab(e.main, QStringLiteral("gallery/lay4-darker-light.png"));
    e.layout->arrangement()->setProperty("tone", QStringLiteral("grey"));
    e.shell->setColorSchemeOverride(Qt::ColorScheme::Dark);
    e.settle();
    e.check("and back", e.main->color() == grey);
    e.shell->setColorSchemeOverride(before);
    e.settle();
}

void galleryPart(Eq &e) {
    const Qt::ColorScheme before = e.shell->colorSchemeOverride();
    e.cc->clearBands(e.out, false);
    e.cc->setBandType(e.out, 0, false, FILTER_PEAKING);
    e.cc->setBandValue(e.out, 0, false, QStringLiteral("freq"), 800);
    e.cc->setBandValue(e.out, 0, false, QStringLiteral("gain"), 5);
    e.cc->setBandType(e.out, 0, true, ChannelController::crossoverType(1, 4, true));
    e.cc->setBandType(e.out + 1, 0, true, ChannelController::crossoverType(1, 4, false));
    e.settle();
    QDir(e.d.outDir()).mkpath(QStringLiteral("gallery"));
    for (const QSize &size : { QSize(1920, 1008), QSize(1280, 800), QSize(1000, 660) }) {
        e.resize(size.width(), size.height());
        for (const char *tab : { "peq", "crossover", "delay" }) {
            e.go(tab);
            e.sc->select(e.out);
            e.settle();
            if (QLatin1String(tab) == QLatin1String("peq")) Registrar::instance<PeqEditorController>()->rowClicked(0, 0);
            for (const bool dark : { true, false }) {
                e.shell->setColorSchemeOverride(dark ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
                e.settle();
                if (PeqOverlay *ov = e.overlay()) ov->settle();
                e.d.spin(60);
                const QString name = QStringLiteral("gallery/lay4-%1-%2-%3.png").arg(size.width()).arg(QLatin1String(tab), dark ? QStringLiteral("dark") : QStringLiteral("light"));
                e.d.check(QStringLiteral("lay4: screenshot %1").arg(name), e.d.grab(e.main, name));
            }
            QQuickItem *scroll = e.d.item(QStringLiteral("PageHost.page.equaliser/%1.scroll").arg(QLatin1String(tab)));
            e.d.check(QStringLiteral("lay4: %1 at %2 px needs no sideways scrolling").arg(QLatin1String(tab)).arg(size.width()),
                      scroll && scroll->property("contentWidth").toReal() <= scroll->width() + 0.5);
            QQuickItem *tools = e.shown("EqualiserTools");
            e.d.check(QStringLiteral("lay4: the row over %1 fits at %2 px").arg(QLatin1String(tab)).arg(size.width()),
                      tools && Eq::inside(tools, e.shown("PageTabs")));
        }
    }
    e.shell->setColorSchemeOverride(before);
    e.cc->clearBands(e.out, false);
    e.cc->clearBands(e.out, true);
    e.cc->clearBands(e.out + 1, true);
    e.resize(1280, 800);
}

void equaliserSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    Eq e{ d, d.app().shell, Registrar::instance<LayoutController>(), Registrar::instance<ChannelController>(),
          Registrar::instance<SidebarController>(), Registrar::instance<GraphController>(), d.mainWindow() };
    e.check("the controllers", e.shell && e.layout && e.cc && e.sc && e.g && e.main);
    if (!e.layout || !e.cc || !e.sc || !e.g || !e.main) return;
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    e.check("on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    // -- FIX016 -- (R1-QT) the speaker layout as it found it: the crossover part's
    // write-back keeps one where there was none (lay5a-pages after it found its
    // Routing inspector 40 px lower, under the dock)
    auto *speakersAt = Registrar::instance<SpeakersController>();
    const QString layoutWas = speakersAt ? speakersAt->status() : QString();
    for (const QString &action : e.shell->shownWindows()) e.shell->closeWindow(action);
    if (e.g->poppedOut()) e.g->closePopOut();
    e.main->requestActivate();
    d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == e.main; });
    const QSize was = e.main->size();
    const QColor classic = e.main->color();
    e.main->resize(1280, 800);
    e.out = e.g->firstOutput();
    e.sc->select(-1);
    e.check("Modern", switchTo(e, QStringLiteral("modern"), "ModernShell") && e.host());
    if (!e.host()) return;
    // The inspector as a new install has it
    e.layout->arrangement()->setProperty("inspector", QStringLiteral("shown"));
    e.resize(1280, 800);

    pagePart(e);
    graphPart(e);
    tablePart(e);
    inspectorPart(e);
    foldPart(e);
    crossoverPart(e);
    delayPart(e);
    clearPart(e);
    dl8Part(e);   // -- WP DL8 --
    tonePart(e);
    galleryPart(e);

    e.sc->select(-1);
    e.check("back in Classic, with its own colours and none of these", switchTo(e, QStringLiteral("classic"), "ClassicShell") && e.main->color() == classic
                && !e.shown("ChannelStrip") && !e.cc->crossoverTabShown());
    e.main->resize(was);
    // -- FIX016 -- (R1-QT) no speaker layout where there was none: the demo opened
    // anew drops the one the write-back kept
    if (speakersAt && layoutWas == QLatin1String("none") && speakersAt->status() != QLatin1String("none")) {
        device->openDemo();
        d.waitFor(10000, [&] { return device->connected() && device->demo() && speakersAt->status() == QLatin1String("none"); });
        d.waitIdle();
    }
    d.spin(200);
}

const SmokeDriver::Section equaliserSmoke("lay4-equaliser", 895, &equaliserSection);

} // namespace
