// -- WP UA4 -- (UXP A4) First run without dead ends, and the Overview's setup
// list, in Modern: the PC words of getting_started.json over the core's
// stages; with no device the wizard reaches Done (the board step's way on);
// Welcome's demo; the Speakers step's one primary (Later, or Confirm and
// Continue after an edit, owner D15); the speaker table's headings, groups
// from 1 and a speaker's name joining its group; the setup list on the
// Overview, its rows, its actions and its close; the layout's outputs as cards
// while nothing is routed. Leaves the layout, the routes, the onboarding flags
// and the layout as it found them.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DeviceRegistry.h"
#include "OnboardingController.h"
#include "Registrar.h"
#include "Shell.h"
#include "SpeakersController.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <cstdio>
#include <functional>

namespace {

QByteArray ua4Fill(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(1 << 16, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

// A core call on the worker, waited for
QJsonObject ua4OnWorker(SmokeDriver &d, std::function<QByteArray(FfiCore *)> job) {
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

QJsonObject ua4Stored(SmokeDriver &d) {
    return ua4OnWorker(d, [](FfiCore *c) { return ua4Fill([c](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); }); });
}

// The layout kept as `layout` says, `confirmed` or not (an import's way, not the app's)
void ua4Keep(SmokeDriver &d, QJsonObject layout, bool confirmed) {
    layout.remove(QStringLiteral("ok"));
    layout.insert(QStringLiteral("confirmedByUser"), confirmed);
    const QByteArray text = QJsonDocument(layout).toJson(QJsonDocument::Compact);
    ua4OnWorker(d, [text](FfiCore *c) {
        return ua4Fill([c, &text](char *b, uint32_t l) { return dspi_speaker_layout_set(c, text.constData(), b, l); });
    });
}

QVariantMap ua4Stage(const QVariantMap &words, const QString &id) {
    for (const QVariant &s : words.value(QStringLiteral("stages")).toList())
        if (s.toMap().value(QStringLiteral("id")).toString() == id) return s.toMap();
    return {};
}

QStringList ua4RowIds(const QVariantMap &stage) {
    QStringList ids;
    for (const QVariant &r : stage.value(QStringLiteral("rows")).toList()) ids << r.toMap().value(QStringLiteral("id")).toString();
    return ids;
}

QString ua4RowText(const QVariantMap &stage, const QString &id) {
    for (const QVariant &r : stage.value(QStringLiteral("rows")).toList())
        if (r.toMap().value(QStringLiteral("id")).toString() == id) return r.toMap().value(QStringLiteral("text")).toString();
    return {};
}

// The texts of the visible items under `from`
void ua4Texts(QQuickItem *from, QStringList *out) {
    if (!from || !from->isVisible()) return;
    const QVariant t = from->property("text");
    if (t.isValid() && !t.toString().isEmpty()) out->append(t.toString());
    for (QQuickItem *c : from->childItems()) ua4Texts(c, out);
}

// The visible item named `name` under `from`
QQuickItem *ua4Shown(QQuickItem *from, const QString &name) {
    if (!from || !from->isVisible()) return nullptr;
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *f = ua4Shown(c, name)) return f;
    return nullptr;
}

QVariantMap ua4Row(SpeakersController *sc, int output) {
    for (const QVariant &v : sc->rows())
        if (v.toMap().value(QStringLiteral("output")).toInt() == output) return v.toMap();
    return {};
}

void ua4Section(SmokeDriver &d) {
    auto check = [&](const char *what, bool ok) { d.check(QStringLiteral("ua4: ") + QString::fromUtf8(what), ok); };
    auto *ob = Registrar::instance<OnboardingController>();
    auto *sc = Registrar::instance<SpeakersController>();
    auto *cc = Registrar::instance<ChannelController>();
    auto *registry = DeviceRegistry::instance();
    DeviceModel *device = d.app().device;
    check("the controllers exist", ob && sc && cc && registry);
    if (!ob || !sc || !cc || !registry) return;
    const bool wasModern = d.app().shell->pageHost() != nullptr;
    check("Modern, for its Overview", d.switchLayout("modern"));
    auto settle = [&] {
        d.spin(60);
        d.waitIdle();
        d.spin(120);
    };
    auto click = [&](QQuickItem *button) {
        const bool ok = SmokeDriver::press(button);
        settle();
        return ok;
    };
    QQuickItem *root = d.mainWindow()->contentItem();
    auto shown = [&](const QString &name) { return ua4Shown(root, name); };

    // ---- The words: the core keeps the PC block on this platform, the app lays it over the stage ----
    const QJsonObject core = QJsonDocument::fromJson(
        ua4Fill([](char *b, uint32_t l) { return dspi_onboarding_getting_started_json(b, l); })).object();
    QJsonObject coreWelcome;
    for (const QJsonValue &s : core.value(QLatin1String("stages")).toArray())
        if (s.toObject().value(QLatin1String("id")).toString() == QLatin1String("welcome")) coreWelcome = s.toObject();
    const QVariantMap words = ob->gettingStarted();
    const QVariantMap welcome = ua4Stage(words, QStringLiteral("welcome"));
    const QVariantMap board = ua4Stage(words, QStringLiteral("board"));
    const QVariantMap done = ua4Stage(words, QStringLiteral("done"));
    check("the core's welcome has its PC block here (windows_or_linux)", coreWelcome.contains(QLatin1String("pc")));
    check("the app's stages have the PC words in place, no block left",
          !welcome.contains(QStringLiteral("pc")) && ua4RowIds(welcome).contains(QStringLiteral("welcome.demo"))
              && ua4RowIds(welcome).contains(QStringLiteral("welcome.agents")) && ua4RowIds(welcome).contains(QStringLiteral("welcome.measure"))
              && !board.value(QStringLiteral("later")).toString().isEmpty() && ua4RowIds(done).contains(QStringLiteral("done.checklist")));

    // ---- No device: the wizard reaches Done ----------------------------------------------------
    for (const QVariant &v : registry->devices()) registry->close(v.toMap().value(QStringLiteral("id")).toString());
    check("no device is open", d.waitFor(8000, [&] { return !device->connected() && registry->devices().isEmpty(); }));
    ob->requestSetup();
    settle();
    QObject *loader = d.object(QStringLiteral("ModernShell.wizard"));
    auto *wizard = loader ? loader->property("item").value<QQuickItem *>() : nullptr;
    check("Getting Started shows", ob->wizardShown() && wizard);
    if (!wizard) return;
    auto at = [&] { return wizard->property("stage").toMap().value(QStringLiteral("id")).toString(); };
    QStringList texts;
    ua4Texts(wizard, &texts);
    check("Welcome says what the app does: EQ, measuring, AI agents",
          texts.contains(ua4RowText(welcome, QStringLiteral("welcome.shape_pc")))
              && texts.contains(ua4RowText(welcome, QStringLiteral("welcome.measure")))
              && texts.contains(ua4RowText(welcome, QStringLiteral("welcome.agents")))
              && texts.contains(ua4RowText(welcome, QStringLiteral("welcome.plug_in"))));
    QQuickItem *demo = ua4Shown(wizard, QStringLiteral("GettingStarted.rowAction.welcome.demo"));
    check("with no device, Welcome offers the demo", demo != nullptr);
    d.grab(d.mainWindow(), QStringLiteral("ua4-welcome.png"));
    click(ua4Shown(wizard, QStringLiteral("GettingStarted.continue")));
    check("the board step waits for a Pico, with no Continue",
          at() == QLatin1String("board") && !ua4Shown(wizard, QStringLiteral("GettingStarted.continue")));
    QQuickItem *later = ua4Shown(wizard, QStringLiteral("GettingStarted.later"));
    check("and a way on: the DSPi already runs the firmware, or comes later",
          later && later->property("text").toString() == board.value(QStringLiteral("later")).toString());
    d.grab(d.mainWindow(), QStringLiteral("ua4-board-later.png"));
    click(later);
    check("which goes on to the Speakers step", at() == QLatin1String("speakers"));
    QQuickItem *go = ua4Shown(wizard, QStringLiteral("GettingStarted.continue"));
    check("with no device it says Continue",
          go && go->property("text").toString() == words.value(QStringLiteral("buttons")).toMap().value(QStringLiteral("continue")).toString());
    click(go);
    check("and Done is reached with no device", at() == QLatin1String("done") && wizard->property("onLast").toBool());
    texts.clear();
    ua4Texts(wizard, &texts);
    check("Done speaks Modern: the Overview's list, the rail, the uConsole menu; not the sidebar or Help",
          texts.contains(ua4RowText(done, QStringLiteral("done.checklist")))
              && texts.contains(ua4RowText(done, QStringLiteral("done.filters_modern")))
              && texts.contains(ua4RowText(done, QStringLiteral("done.help_modern")))
              && !texts.contains(ua4RowText(done, QStringLiteral("done.filters_classic")))
              && !texts.contains(ua4RowText(done, QStringLiteral("done.help_classic"))));
    check("Done offers its sound output check (Settings › This Computer)", ua4Shown(wizard, QStringLiteral("GettingStarted.rowAction.done.sound_pc")) != nullptr);
    d.grab(d.mainWindow(), QStringLiteral("ua4-done.png"));

    // ---- Welcome's demo -------------------------------------------------------------------
    wizard->setProperty("current", 0);
    settle();
    click(ua4Shown(wizard, QStringLiteral("GettingStarted.rowAction.welcome.demo")));
    check("Welcome's button opens the demo", d.waitFor(15000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    settle();
    check("and offers it no more", !ua4Shown(wizard, QStringLiteral("GettingStarted.rowAction.welcome.demo")));

    // ---- The Speakers step: Later, then Confirm and Continue --------------------------------
    sc->reload();
    d.waitFor(5000, [&] { return sc->loaded(); });
    const QJsonObject found = ua4Stored(d);
    const bool foundConfirmed = found.value(QLatin1String("confirmedByUser")).toBool();
    // An unconfirmed layout, as an import leaves it
    QJsonObject base = found;
    if (!base.contains(QLatin1String("outputs")) || base.value(QLatin1String("value")).isNull())
        base = ua4OnWorker(d, [](FfiCore *c) { return ua4Fill([c](char *b, uint32_t l) { return dspi_speaker_layout_propose(c, b, l); }); });
    ua4Keep(d, base, false);
    sc->reload();
    check("the layout is not confirmed", d.waitFor(5000, [&] { return sc->loaded() && sc->status() != QLatin1String("confirmed"); }));
    const QVariantList stages = words.value(QStringLiteral("stages")).toList();
    int speakersAt = -1;
    for (int i = 0; i < stages.size(); ++i)
        if (stages[i].toMap().value(QStringLiteral("id")).toString() == QLatin1String("speakers")) speakersAt = i;
    wizard->setProperty("current", speakersAt);
    settle();
    const QVariantMap speakersButtons = ua4Stage(words, QStringLiteral("speakers")).value(QStringLiteral("buttons")).toMap();
    go = ua4Shown(wizard, QStringLiteral("GettingStarted.continue"));
    QQuickItem *confirm = ua4Shown(wizard, QStringLiteral("SpeakersStep.confirm"));
    check("an untouched table: the one primary is Later",
          go && go->property("text").toString() == speakersButtons.value(QStringLiteral("later")).toString()
              && go->property("primary").toBool() && confirm && !confirm->property("primary").toBool());
    check("it says what the layout is needed for",
          ua4Shown(wizard, QStringLiteral("SpeakersStep.later"))
              && ua4Shown(wizard, QStringLiteral("SpeakersStep.later"))->property("text").toString().contains(QLatin1String("AI agents")));

    // The table: headings, groups from 1, a speaker's name joins its group
    check("the table has its headings", ua4Shown(wizard, QStringLiteral("Speakers.headings")) != nullptr);
    int first = -1, other = -1;
    for (const QVariant &v : sc->rows()) {
        const QVariantMap r = v.toMap();
        if (!r.value(QStringLiteral("included")).toBool()) continue;
        if (first < 0) first = r.value(QStringLiteral("output")).toInt();
        else if (other < 0 && r.value(QStringLiteral("group")).toInt() != ua4Row(sc, first).value(QStringLiteral("group")).toInt())
            other = r.value(QStringLiteral("output")).toInt();
    }
    QQuickItem *groupBox = ua4Shown(wizard, QStringLiteral("Speakers.group.%1").arg(first));
    check("a group shows counted from 1",
          first >= 0 && groupBox && qRound(groupBox->property("realValue").toDouble()) == ua4Row(sc, first).value(QStringLiteral("group")).toInt() + 1);
    d.grab(d.mainWindow(), QStringLiteral("ua4-speakers-later.png"));
    if (other < 0) {
        // One group only: give the second row a group of its own
        for (const QVariant &v : sc->rows())
            if (v.toMap().value(QStringLiteral("included")).toBool() && v.toMap().value(QStringLiteral("output")).toInt() != first) {
                other = v.toMap().value(QStringLiteral("output")).toInt();
                sc->setGroup(other, ua4Row(sc, first).value(QStringLiteral("group")).toInt() + 1);
                break;
            }
        settle();
    }
    click(go);
    check("Later goes on and leaves the layout unconfirmed",
          at() == QLatin1String("done") && sc->status() != QLatin1String("confirmed"));
    wizard->setProperty("current", speakersAt);
    settle();
    if (other >= 0) {
        const QString name = QStringLiteral("UA4 speaker");
        sc->setSpeaker(other, name);
        settle();
        QQuickItem *field = ua4Shown(wizard, QStringLiteral("Speakers.speaker.%1").arg(first));
        if (field) QMetaObject::invokeMethod(field, "committed", Q_ARG(QString, name));
        settle();
        check("a speaker named like another row's joins its group",
              field && ua4Row(sc, first).value(QStringLiteral("group")) == ua4Row(sc, other).value(QStringLiteral("group")));
    }
    go = ua4Shown(wizard, QStringLiteral("GettingStarted.continue"));
    check("an edited table: the one primary is Confirm and Continue",
          go && go->property("text").toString() == speakersButtons.value(QStringLiteral("confirm_continue")).toString());
    d.grab(d.mainWindow(), QStringLiteral("ua4-speakers-confirm.png"));
    click(go);
    check("which keeps the layout, confirmed, and goes on",
          d.waitFor(5000, [&] { return at() == QLatin1String("done"); }) && sc->status() == QLatin1String("confirmed") && !sc->edited());
    click(ua4Shown(wizard, QStringLiteral("GettingStarted.rowAction.done.sound_pc")));
    check("Done's This Computer finishes setup", !ob->wizardShown() && ob->setupDone());
    settle();

    // ---- The Overview's setup list -------------------------------------------------------------
    ua4Keep(d, base, false);
    sc->reload();
    d.waitFor(5000, [&] { return sc->loaded() && sc->status() != QLatin1String("confirmed"); });
    d.goTo(QStringLiteral("overview/overview"), 300);
    QQuickItem *setup = shown(QStringLiteral("OverviewChecklist"));
    check("an unconfirmed layout: the Overview shows the setup list", setup != nullptr);
    auto rowDone = [&](const QString &key) {
        QQuickItem *row = shown(QStringLiteral("OverviewChecklist.row.") + key);
        return row && row->property("done").toBool();
    };
    check("the speakers row is open, with its action",
          shown(QStringLiteral("OverviewChecklist.row.speakers")) && !rowDone(QStringLiteral("speakers"))
              && shown(QStringLiteral("OverviewChecklist.action.speakers")));
    check("the routing row is done on the demo, without an action",
          rowDone(QStringLiteral("routing")) && !shown(QStringLiteral("OverviewChecklist.action.routing")));
    check("it counts what is done", shown(QStringLiteral("OverviewChecklist.count"))
                                        && shown(QStringLiteral("OverviewChecklist.count"))->property("text").toString().contains(QLatin1String(" of ")));
    d.grab(d.mainWindow(), QStringLiteral("ua4-overview-checklist.png"));
    // (an earlier section may have measured: then the row is done)
    if (!rowDone(QStringLiteral("measure"))) {
        QQuickItem *host = qobject_cast<QQuickItem *>(d.app().shell->pageHost());
        click(shown(QStringLiteral("OverviewChecklist.action.measure")));
        check("Measure goes to the Measure page", host && host->property("section").toString() == QLatin1String("measure"));
        d.goTo(QStringLiteral("overview/overview"), 300);
    }

    // Nothing routed: the layout's outputs as cards that say so
    struct Route { int i, o; bool on; double gain; bool invert; };
    QList<Route> routes;
    for (int i = 0; i < cc->numInputs(); ++i)
        for (int o = 0; o < cc->numOutputs(); ++o) {
            const QVariantMap r = cc->route(i, o);
            routes.append({ i, o, r.value(QStringLiteral("enabled")).toBool(), r.value(QStringLiteral("gain")).toDouble(),
                            r.value(QStringLiteral("invert")).toBool() });
        }
    for (const Route &r : routes)
        if (r.on) cc->setRoute(r.i, r.o, false, r.gain, r.invert);
    settle();
    d.spin(400);
    check("nothing routed: the routing row is open", !rowDone(QStringLiteral("routing")) && shown(QStringLiteral("OverviewChecklist.action.routing")));
    check("the layout's outputs show, said not to be routed",
          d.waitFor(3000, [&] { return shown(QStringLiteral("OverviewSection.notRouted")) != nullptr; })
              && shown(QStringLiteral("Dashboard.open.%1").arg(cc->outputChannel(first))));
    d.grab(d.mainWindow(), QStringLiteral("ua4-overview-not-routed.png"));
    // No output in the layout either: OUTPUTS stays, its notice says what to do
    QList<int> included;
    for (const QVariant &v : sc->rows())
        if (v.toMap().value(QStringLiteral("included")).toBool()) included << v.toMap().value(QStringLiteral("output")).toInt();
    for (int o : included) sc->setIncluded(o, false);
    settle();
    QQuickItem *notice = shown(QStringLiteral("OverviewSection.notRouted"));
    check("no layout and nothing routed: OUTPUTS says to route an input",
          shown(QStringLiteral("OverviewSection.outputs")) && notice
              && notice->property("text").toString().startsWith(QLatin1String("No output is in use")));
    for (int o : included) sc->setIncluded(o, true);
    settle();
    for (const Route &r : routes)
        if (r.on) cc->setRoute(r.i, r.o, true, r.gain, r.invert);
    settle();
    check("routed again: no such notice", d.waitFor(3000, [&] { return !shown(QStringLiteral("OverviewSection.notRouted")); }));

    // A confirmed layout ticks its row; closing the list keeps it closed
    sc->confirm();
    check("Confirm ticks the speakers row", d.waitFor(5000, [&] { return rowDone(QStringLiteral("speakers")); }));
    click(shown(QStringLiteral("OverviewChecklist.close")));
    check("closed, the list goes, for this device", !shown(QStringLiteral("OverviewChecklist")) && ob->checklistClosed(device->serial()));
    check("another device has its own", !ob->checklistClosed(QStringLiteral("UA4-OTHER")));

    // ---- Left as found ------------------------------------------------------------------------
    ob->resetOnboarding();
    ob->finishSetup();
    ob->markWhatsNewRead();
    if (!found.value(QLatin1String("value")).isNull() && found.contains(QLatin1String("outputs"))) ua4Keep(d, found, foundConfirmed);
    sc->reload();
    d.waitFor(5000, [&] { return sc->loaded(); });
    if (!wasModern) d.switchLayout("classic");
    check("left as found", ob->setupDone() && !ob->checklistClosed(device->serial()) && device->connected());
}

} // namespace

// After roleq-speakers (the same order, by name), which starts from a fresh layout
const SmokeDriver::Section ua4("ua4-firstrun", 205, &ua4Section);
