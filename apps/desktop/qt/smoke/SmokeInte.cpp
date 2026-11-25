// -- WP INTE -- The integration's fixes on the Qt client, on the demo device:
// - "inte-unused": (owner 2026-11-24) an output nothing is routed to is left
//   out of the views that only show: the dock's meters, the spectrum's channel
//   chooser (one being analysed stays, so it can be turned off) and the Measure
//   Live tab's outputs; the full list (the Setup tab, an edit view) keeps it.
// - "inte-typed": (REVE C1) Enter in a number field with nothing typed writes
//   nothing and shows the value again; typed, it writes.
// - "inte-words": (SHOWFIX leftovers) the Equaliser's header and inspector and
//   the Measure form name an output by the core's channel words ("Centre
//   woofer", not "Out 1 · ..."), and the Equaliser's list shows that name whole.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "MeasureController.h"
#include "Registrar.h"
#include "RtaController.h"
#include "SidebarController.h"
#include "Snapshot.h"

#include <QGuiApplication>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QTest>
#include <QtQml/QQmlApplicationEngine>
#include <cstdio>
#include <functional>

namespace {

// The shown item named `name` under `from`
QQuickItem *shownItem(QQuickItem *from, const QString &name) {
    if (!from) return nullptr;
    if (from->objectName() == name && from->isVisible()) return from;
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children)
        if (QQuickItem *found = shownItem(child, name)) return found;
    return nullptr;
}

// A Text under `from` that shows `text`
QQuickItem *textItem(QQuickItem *from, const QString &text) {
    if (!from) return nullptr;
    if (from->inherits("QQuickText") && from->property("text").toString() == text) return from;
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children)
        if (QQuickItem *found = textItem(child, text)) return found;
    return nullptr;
}

QString textOf(QQuickItem *i) { return i ? i->property("text").toString() : QString(); }

QByteArray fill(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(1 << 16, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

// A core call on the active device's worker, waited for (SmokeMcp5's)
QJsonObject onWorker(SmokeDriver &d, std::function<QByteArray(FfiCore *)> job) {
    QByteArray out;
    bool done = false;
    QObject ctx;
    d.app().core->worker()->call(&ctx, std::move(job), [&](const QByteArray &r) {
        out = r;
        done = true;
    });
    d.waitFor(5000, [&] { return done; });
    return QJsonDocument::fromJson(out).object();
}

// A wire channel's row in the channel list: "unused", "displayName", ...
QVariant rowValue(DeviceModel *device, int channel, const char *role) {
    ChannelListModel *rows = device->channels();
    const int row = rows ? rows->rowOfChannel(channel) : -1;
    return row >= 0 ? rows->get(row, QString::fromLatin1(role)) : QVariant();
}

bool listsChannel(const QVariantList &list, const char *key, int value) {
    for (const QVariant &v : list)
        if (v.toMap().value(QString::fromLatin1(key)).toInt() == value) return true;
    return false;
}

// ---- inte-unused ---------------------------------------------------------------

struct Route {
    bool on = false;
    double gain = 0;
    bool invert = false;
};

void unusedSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    auto *cc = Registrar::instance<ChannelController>();
    auto *sc = Registrar::instance<SidebarController>();
    auto *rta = Registrar::instance<RtaController>();
    auto *mc = Registrar::instance<MeasureController>();
    d.check("inte: the controllers", cc && sc && rta && mc);
    if (!cc || !sc || !rta || !mc) return;
    d.check("inte: on the demo", d.waitFor(10000, [&] { return device->connected(); }) && d.waitIdle());
    d.check("inte: in Modern (the dock)", d.switchLayout("modern"));
    sc->select(-1);
    d.goTo(QStringLiteral("overview/overview"), 300);
    d.check("inte: the analyser", d.waitFor(10000, [&] { return rta->available(); }));
    const auto s = [&] { return device->state(); };
    const int inputs = cc->numInputs();
    const int outputs = cc->numOutputs();
    const auto routed = [&](int o) {
        for (int i = 0; i < inputs; ++i)
            if (s() && s()->matrix_routing[i][o]) return true;
        return false;
    };
    const auto offered = [&](int o) { return listsChannel(rta->channels(RTA_TAP_OUTPUT), "channel", o); };
    // One output being analysed (it stays offered) and one not (it goes)
    const bool outputSide = rta->drawnTap() == RTA_TAP_OUTPUT;
    const QList<int> drawn = rta->drawnChannels();
    int kept = -1, hidden = -1;
    for (int o = 0; o < outputs && s(); ++o) {
        if (!s()->output_enabled[o] || !routed(o) || !offered(o)) continue;
        if (outputSide && drawn.contains(o)) {
            if (kept < 0) kept = o;
        } else if (hidden < 0) {
            hidden = o;
        }
    }
    d.check(QStringLiteral("inte: an enabled output with routes that is not analysed (%1)").arg(hidden), hidden >= 0);
    if (hidden < 0) {
        d.switchLayout("classic");
        return;
    }
    const QString meter = QStringLiteral("Dock.meter.%1").arg(inputs + hidden);
    QQuickItem *before = d.item(meter);
    d.check("inte: its meter shows in the dock", before && before->isVisible());
    d.check("inte: the spectrum offers it", offered(hidden));
    d.check("inte: Measure's Live tab offers it", listsChannel(mc->liveOutputs(), "output", hidden));

    // Every route into it (and into the analysed one) off
    QVector<QVector<Route>> was(outputs, QVector<Route>(inputs));
    const QList<int> changed = kept >= 0 ? QList<int>{ hidden, kept } : QList<int>{ hidden };
    for (int o : changed)
        for (int i = 0; i < inputs; ++i) {
            was[o][i] = { s()->matrix_routing[i][o], double(s()->matrix_gain[i][o]), s()->matrix_invert[i][o] };
            if (was[o][i].on) cc->setRoute(i, o, false, 0, false);
        }
    d.waitIdle();
    d.check("inte: with nothing routed to it, the core calls it unused",
            d.waitFor(5000, [&] { return rowValue(device, cc->outputChannel(hidden), "unused").toBool(); }));
    d.spin(200);
    QQuickItem *after = d.item(meter);
    d.check("inte: the dock leaves its meter out (a view that only shows)", after && !after->isVisible());
    d.check("inte: the spectrum's chooser leaves it out", d.waitFor(2000, [&] { return !offered(hidden); }));
    if (kept >= 0)
        d.check("inte: one being analysed stays offered, so it can be turned off",
                d.waitFor(2000, [&] { return rowValue(device, cc->outputChannel(kept), "unused").toBool(); }) && offered(kept));
    else
        d.notRun("inte: one being analysed stays offered", QStringLiteral("no output with routes is being analysed"));
    d.check("inte: Measure's Live tab leaves it out, the Setup tab's list keeps it",
            d.waitFor(3000, [&] { return !listsChannel(mc->liveOutputs(), "output", hidden); })
                && listsChannel(mc->outputs(), "output", hidden) && listsChannel(mc->setupOutputs(), "output", hidden));
    d.check("inte: the channel list (an edit view) keeps its row", device->channels()->rowOfChannel(inputs + hidden) >= 0);

    // Back as it was
    for (int o : changed)
        for (int i = 0; i < inputs; ++i)
            if (was[o][i].on) cc->setRoute(i, o, true, was[o][i].gain, was[o][i].invert);
    d.waitIdle();
    d.check("inte: routed again, its meter is back", d.waitFor(5000, [&] {
        QQuickItem *m = d.item(meter);
        return !rowValue(device, cc->outputChannel(hidden), "unused").toBool() && m && m->isVisible();
    }));
    d.check("inte: and the spectrum offers it again", d.waitFor(2000, [&] { return offered(hidden); }));
    d.switchLayout("classic");
}

// ---- inte-typed ----------------------------------------------------------------

// A QML object of module UConsole made for a test, in the main window (SmokePolish's)
QQuickItem *make(SmokeDriver &d, const char *type, const QVariantMap &props) {
    QQmlComponent c(d.app().engine, QStringLiteral("UConsole"), QString::fromLatin1(type));
    QVariantMap p = props;
    p.insert(QStringLiteral("parent"), QVariant::fromValue(d.mainWindow()->contentItem()));
    QObject *o = c.createWithInitialProperties(p);
    if (!o) std::printf("smoke: inte: %s did not load: %s\n", type, qPrintable(c.errorString()));
    auto *item = qobject_cast<QQuickItem *>(o);
    if (o && !item) delete o;
    return item;
}

void typedSection(SmokeDriver &d) {
    QQuickWindow *w = d.mainWindow();
    d.check("inte: the main window", w != nullptr);
    if (!w) return;
    w->requestActivate();
    d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == w; });

    // NumericField: 1.26 dB shows as "1.3 dB"; Enter wrote that 1.3 back
    QQuickItem *n = make(d, "NumericField", { { QStringLiteral("unit"), QStringLiteral("dB") }, { QStringLiteral("from"), -24 },
                                              { QStringLiteral("to"), 24 }, { QStringLiteral("decimals"), 1 },
                                              { QStringLiteral("value"), 1.26 } });
    QQuickItem *input = n ? n->findChild<QQuickItem *>(QStringLiteral("NumericField.input")) : nullptr;
    d.check("inte: a NumericField", n && input);
    if (n && input) {
        QSignalSpy commits(n, SIGNAL(committed(double)));
        input->forceActiveFocus();
        d.spin(30);
        QTest::keyClick(w, Qt::Key_Return);
        d.spin(30);
        d.check(QStringLiteral("inte: NumericField: Enter with nothing typed writes nothing (%1 writes)").arg(commits.count()),
                commits.isEmpty());
        d.check(QStringLiteral("inte: NumericField: and shows the value again (%1)").arg(textOf(input)),
                textOf(input) == QLatin1String("1.3 dB"));
        QTest::keyClick(w, Qt::Key_2);   // all selected after Enter: replaces it
        QTest::keyClick(w, Qt::Key_Return);
        d.spin(30);
        d.check("inte: NumericField: typed, Enter writes it",
                commits.count() == 1 && qAbs(commits.first().value(0).toDouble() - 2) < 1e-9);
        delete n;
    }

    // BorderedField (MatrixField, ChannelValueField and the renames build on it)
    QQuickItem *b = make(d, "BorderedField", { { QStringLiteral("text"), QStringLiteral("1.25") } });
    d.check("inte: a BorderedField", b != nullptr);
    if (b) {
        QSignalSpy commits(b, SIGNAL(committed(QString)));
        QSignalSpy untyped(b, SIGNAL(enteredUntyped()));
        b->forceActiveFocus();
        d.spin(30);
        QTest::keyClick(w, Qt::Key_Return);
        d.spin(30);
        d.check(QStringLiteral("inte: BorderedField: Enter with nothing typed commits nothing (%1 commits)").arg(commits.count()),
                commits.isEmpty() && untyped.count() == 1 && textOf(b) == QLatin1String("1.25"));
        QTest::keyClick(w, Qt::Key_A, Qt::ControlModifier);
        QTest::keyClick(w, Qt::Key_3);
        QTest::keyClick(w, Qt::Key_Return);
        d.spin(30);
        d.check("inte: BorderedField: typed, Enter commits it",
                commits.count() == 1 && commits.first().value(0).toString() == QLatin1String("3"));
        delete b;
    }
}

// ---- inte-words ----------------------------------------------------------------

void wordsSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    auto *cc = Registrar::instance<ChannelController>();
    auto *sc = Registrar::instance<SidebarController>();
    auto *mc = Registrar::instance<MeasureController>();
    QQuickWindow *w = d.mainWindow();
    d.check("inte: the controllers and the window", cc && sc && mc && w);
    if (!cc || !sc || !mc || !w) return;
    d.check("inte: on the demo", d.waitFor(10000, [&] { return device->connected(); }) && d.waitIdle());
    // Output 1 is the centre speaker's woofer in the speaker layout, someone's word
    const QJsonValue stored = onWorker(d, [](FfiCore *c) {
                                  return fill([c](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); });
                              }).value(QLatin1String("value"));
    const QJsonObject layout{ { QStringLiteral("outputs"),
                                QJsonArray{ QJsonObject{ { QStringLiteral("output"), 0 },
                                                         { QStringLiteral("role"), QStringLiteral("low") },
                                                         { QStringLiteral("speaker"), QStringLiteral("C") },
                                                         { QStringLiteral("group"), 0 } } } },
                              { QStringLiteral("confirmedByUser"), true } };
    const auto setLayout = [&](const QJsonObject &l) {
        const QByteArray text = QJsonDocument(l).toJson(QJsonDocument::Compact);
        return onWorker(d, [text](FfiCore *c) {
            return fill([c, &text](char *b, uint32_t n) { return dspi_speaker_layout_set(c, text.constData(), b, n); });
        });
    };
    setLayout(layout);
    device->refresh();
    const auto fromLayout = [&] {
        return device->snapshot().channelWords.value(QLatin1String("fromLayout")).toArray().contains(QJsonValue(0));
    };
    d.check("inte: the core names output 1 from the layout", d.waitFor(8000, fromLayout));
    const QString word = rowValue(device, cc->outputChannel(0), "shortName").toString();
    const QString label = rowValue(device, cc->outputChannel(0), "label").toString();
    d.check(QStringLiteral("inte: its channel word (%1) is not the core's label (%2)").arg(word, label),
            !word.isEmpty() && word != label && rowValue(device, cc->outputChannel(0), "displayName").toString() == word);

    // The Measure form's rows
    d.check(QStringLiteral("inte: the Measure form's row names it %1").arg(word), d.waitFor(3000, [&] {
        for (const QVariant &v : mc->setupOutputs())
            if (v.toMap().value(QStringLiteral("output")).toInt() == 0) return v.toMap().value(QStringLiteral("label")).toString() == word;
        return false;
    }));

    // The Equaliser: its header, its inspector and its list
    const QSize original = w->size();
    if (d.switchLayout("modern")) {
        w->resize(QSize(1600, 1000));
        d.goTo(QStringLiteral("equaliser/peq"), 300);
        sc->select(cc->outputChannel(0));
        d.spin(200);
        d.waitIdle();
        QQuickItem *header = shownItem(w->contentItem(), QStringLiteral("ChannelStrip.name"));
        d.check(QStringLiteral("inte: the Equaliser's header names it %1 (%2)").arg(word, textOf(header)), textOf(header) == word);
        if (QObject *state = d.app().engine->singletonInstance<QObject *>("UConsole", "InspectorState")) {
            QVariant r;
            QMetaObject::invokeMethod(state, "show", Q_RETURN_ARG(QVariant, r));
        }
        d.spin(300);
        QQuickItem *title = shownItem(w->contentItem(), QStringLiteral("ChannelInspector.title"));
        if (title)
            d.check(QStringLiteral("inte: the Equaliser's inspector names it %1 (%2)").arg(word, textOf(title)), textOf(title) == word);
        else
            d.notRun("inte: the Equaliser's inspector names it", QStringLiteral("the inspector did not show"));
        QQuickItem *list = shownItem(w->contentItem(), QStringLiteral("EqualiserPeq.channels"));
        QQuickItem *row = shownItem(list, QStringLiteral("Sidebar.channel.%1").arg(cc->outputChannel(0)));
        QQuickItem *name = textItem(row, word);
        d.check(QStringLiteral("inte: the Equaliser's list shows %1 whole").arg(word), name && !name->property("truncated").toBool());
        w->resize(original);
        sc->select(-1);
        d.switchLayout("classic");
    } else {
        d.check("inte: in Modern", false);
    }

    // The layout as it was (none stays: a layout cannot be unset)
    if (stored.isObject()) {
        setLayout(stored.toObject());
        device->refresh();
        d.waitIdle();
    }
}

const SmokeDriver::Section unused("inte-unused", 989, &unusedSection);
const SmokeDriver::Section typed("inte-typed", 989, &typedSection);
const SmokeDriver::Section words("inte-words", 989, &wordsSection);

} // namespace
