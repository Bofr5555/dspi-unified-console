// -- WP UB7 -- (UXP B7, C3) One speaker table, shown once, edited in one place,
// in Modern: Settings › Speakers' table with the heading "Measured together",
// the glossary's role words and one word for the suggestion; a group that
// follows the speaker's name (the same name joins, a new name stands alone),
// shown as the names; the override ("Choose which outputs are measured
// together") and, off again, the groups back to the names; a Detect… that
// cannot start says why under it. Measure and Auto-tune: the layout in one
// line with Edit in Speakers…, the outputs as tick boxes only (no role,
// speaker or group to edit there); a tick is this run's, the layout keeps.
// Leaves the layout and the layout mode as it found them.
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "MeasureController.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SpeakersController.h"
#include "TuneController.h"

#include <QJSValue>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <cstdio>
#include <functional>

namespace {

QByteArray ub7Fill(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(1 << 16, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

QJsonObject ub7OnWorker(SmokeDriver &d, std::function<QByteArray(FfiCore *)> job) {
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

QJsonObject ub7Stored(SmokeDriver &d) {
    return ub7OnWorker(d, [](FfiCore *c) { return ub7Fill([c](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); }); });
}

void ub7Keep(SmokeDriver &d, QJsonObject layout, bool confirmed) {
    layout.remove(QStringLiteral("ok"));
    layout.insert(QStringLiteral("confirmedByUser"), confirmed);
    const QByteArray text = QJsonDocument(layout).toJson(QJsonDocument::Compact);
    ub7OnWorker(d, [text](FfiCore *c) {
        return ub7Fill([c, &text](char *b, uint32_t l) { return dspi_speaker_layout_set(c, text.constData(), b, l); });
    });
}

QQuickItem *ub7Shown(QQuickItem *from, const QString &name) {
    if (!from || !from->isVisible()) return nullptr;
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *f = ub7Shown(c, name)) return f;
    return nullptr;
}

// The visible items under `from` whose `label` (a control's accessible name) starts with `prefix`
int ub7Labelled(QQuickItem *from, const QString &prefix) {
    if (!from || !from->isVisible()) return 0;
    int n = from->property("label").toString().startsWith(prefix) ? 1 : 0;
    for (QQuickItem *c : from->childItems()) n += ub7Labelled(c, prefix);
    return n;
}

QStringList ub7Texts(QQuickItem *from) {
    QStringList out;
    if (!from || !from->isVisible()) return out;
    if (const QVariant t = from->property("text"); t.isValid() && !t.toString().isEmpty()) out << t.toString();
    for (QQuickItem *c : from->childItems()) out << ub7Texts(c);
    return out;
}

// A QML model (a JS array) as a list
QVariantList ub7List(const QVariant &v) {
    return v.canConvert<QJSValue>() ? v.value<QJSValue>().toVariant().toList() : v.toList();
}

QVariantMap ub7Row(SpeakersController *sc, int output) {
    for (const QVariant &v : sc->rows())
        if (v.toMap().value(QStringLiteral("output")).toInt() == output) return v.toMap();
    return {};
}

// A check box clicked: it toggles and says so (press() only clicks)
void ub7Toggle(QObject *box) {
    if (!box) return;
    QMetaObject::invokeMethod(box, "toggle");
    QMetaObject::invokeMethod(box, "toggled");
}

int ub7Group(SpeakersController *sc, int output) { return ub7Row(sc, output).value(QStringLiteral("group")).toInt(); }

void ub7Section(SmokeDriver &d) {
    auto check = [&](const char *what, bool ok) { d.check(QStringLiteral("ub7: ") + QString::fromUtf8(what), ok); };
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo()) device->openDemo();
    check("the demo DSPi is open", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    auto *sc = Registrar::instance<SpeakersController>();
    auto *mc = Registrar::instance<MeasureController>();
    auto *tc = Registrar::instance<TuneController>();
    auto *settingsCtl = Registrar::instance<SettingsController>();
    check("the controllers exist", sc && mc && tc && settingsCtl);
    if (!sc || !mc || !tc || !settingsCtl) return;
    const bool wasModern = d.app().shell->pageHost() != nullptr;
    check("Modern", d.switchLayout("modern"));
    auto settle = [&] {
        d.spin(60);
        d.waitIdle();
        d.spin(120);
    };
    const QJsonObject found = ub7Stored(d);
    const bool foundConfirmed = found.value(QLatin1String("confirmedByUser")).toBool();
    sc->reload();
    check("the layout is read", d.waitFor(5000, [&] { return sc->loaded(); }));
    // A confirmed layout of the suggestion to start from
    sc->useSuggestion();
    sc->confirm();
    check("a confirmed layout to start from", d.waitFor(5000, [&] { return sc->status() == QLatin1String("confirmed") && !sc->edited(); }));
    QList<int> in;
    for (const QVariant &v : sc->rows())
        if (v.toMap().value(QStringLiteral("included")).toBool()) in << v.toMap().value(QStringLiteral("output")).toInt();
    check("the suggestion has two outputs or more", in.size() >= 2);
    if (in.size() < 2) return;

    // ---- Settings › Speakers: the one table ---------------------------------------------------
    sc->openSettings();
    auto *settings = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("edit.settings")));
    check("Settings › Speakers opens",
          settings && d.waitFor(3000, [&] { return settings->isVisible() && ub7Shown(settings->contentItem(), QStringLiteral("SpeakersPage")); }));
    QQuickItem *page = settings ? ub7Shown(settings->contentItem(), QStringLiteral("SpeakersPage")) : nullptr;
    auto shown = [&](const QString &name) { return ub7Shown(page, name); };
    const QStringList headings = ub7Texts(shown(QStringLiteral("Speakers.headings")));
    check("the headings: Output, Role, Speaker, Measured together",
          headings == QStringList({ QStringLiteral("OUTPUT"), QStringLiteral("ROLE"), QStringLiteral("SPEAKER"), QStringLiteral("MEASURED TOGETHER") }));
    std::printf("smoke: ub7: headings %s\n", qPrintable(headings.join(QStringLiteral(" | "))));
    QQuickItem *role = shown(QStringLiteral("Speakers.role.%1").arg(in.first()));
    QStringList roleWords;
    if (role)
        for (const QVariant &r : ub7List(role->property("model"))) roleWords << r.toMap().value(QStringLiteral("text")).toString();
    check("the roles in the glossary's words",
          roleWords == QStringList({ QStringLiteral("Full range"), QStringLiteral("Subwoofer"), QStringLiteral("Woofer"),
                                     QStringLiteral("Mid driver"), QStringLiteral("Tweeter") }));
    QQuickItem *suggest = shown(QStringLiteral("SpeakersPage.suggestion"));
    check("one word for the suggestion", suggest && suggest->property("text").toString() == QLatin1String("Suggest from Routing and Filters"));

    // The detection cannot start over a confirmed layout: the caption says why
    QQuickItem *detect = shown(QStringLiteral("SpeakersPage.detect"));
    QQuickItem *why = shown(QStringLiteral("SpeakersPage.detectWhy"));
    check("a Detect… that cannot start says why under it",
          detect && !detect->isEnabled() && !sc->detectBlocker().isEmpty() && why && why->property("text").toString() == sc->detectBlocker());

    // Groups follow the names, shown as the names
    if (!sc->groupsFollowNames()) {
        sc->followNames();
        settle();
    }
    check("the groups follow the names", sc->groupsFollowNames());
    const int a = in.at(0);
    int b = -1;
    for (int o : in)
        if (o != a && ub7Row(sc, o).value(QStringLiteral("speaker")).toString().trimmed().compare(
                          ub7Row(sc, a).value(QStringLiteral("speaker")).toString().trimmed(), Qt::CaseInsensitive) != 0) {
            b = o;
            break;
        }
    if (b < 0) {   // one speaker: the other output gets a name of its own first
        b = in.at(1);
        sc->setSpeaker(b, QStringLiteral("UB7 other"));
        settle();
    }
    const QString nameA = ub7Row(sc, a).value(QStringLiteral("speaker")).toString().trimmed();
    QQuickItem *togetherA = shown(QStringLiteral("Speakers.together.%1").arg(a));
    check("measured together shows the names, not a number",
          togetherA && !nameA.isEmpty() && togetherA->property("text").toString() == ub7Row(sc, a).value(QStringLiteral("groupName")).toString()
              && togetherA->property("text").toString().contains(nameA));
    check("no group picker while the groups are the names'", !shown(QStringLiteral("Speakers.group.%1").arg(a)));
    check("two names, two groups", ub7Group(sc, a) != ub7Group(sc, b));
    QQuickItem *fieldB = shown(QStringLiteral("Speakers.speaker.%1").arg(b));
    if (fieldB) QMetaObject::invokeMethod(fieldB, "committed", Q_ARG(QString, nameA.toUpper()));
    settle();
    check("a speaker named like another row's joins its group", fieldB && ub7Group(sc, b) == ub7Group(sc, a));
    fieldB = shown(QStringLiteral("Speakers.speaker.%1").arg(b));
    if (fieldB) QMetaObject::invokeMethod(fieldB, "committed", Q_ARG(QString, QStringLiteral("UB7 new")));
    settle();
    bool alone = true;
    for (int o : in)
        if (o != b && ub7Group(sc, o) == ub7Group(sc, b)) alone = false;
    check("a new name leaves the group it shared for one of its own", fieldB && alone && sc->groupsFollowNames());
    std::printf("smoke: ub7: a=%d g%d \"%s\", b=%d g%d \"%s\"\n", a, ub7Group(sc, a), qPrintable(ub7Row(sc, a).value(QStringLiteral("groupName")).toString()), b,
                ub7Group(sc, b), qPrintable(ub7Row(sc, b).value(QStringLiteral("groupName")).toString()));
    d.grab(settings, QStringLiteral("ub7-speakers.png"));

    // The override, and off again
    QQuickItem *byHand = shown(QStringLiteral("Speakers.byHand"));
    check("the override is offered, off", byHand && !byHand->property("checked").toBool());
    ub7Toggle(byHand);
    settle();
    QQuickItem *pickB = shown(QStringLiteral("Speakers.group.%1").arg(b));
    check("on, each row picks what it is measured with", byHand && byHand->property("checked").toBool() && pickB
                                                           && !shown(QStringLiteral("Speakers.together.%1").arg(b)));
    if (pickB) {
        const QVariantList choices = ub7List(pickB->property("model"));
        int withA = -1;
        for (int i = 0; i < choices.size(); ++i)
            if (choices[i].toMap().value(QStringLiteral("group")).toInt() == ub7Group(sc, a) && !choices[i].toMap().value(QStringLiteral("alone")).toBool()) withA = i;
        if (withA >= 0) QMetaObject::invokeMethod(pickB, "activated", Q_ARG(int, withA));
        settle();
        check("a pick puts it in the other speaker's group", withA >= 0 && ub7Group(sc, b) == ub7Group(sc, a) && !sc->groupsFollowNames());
        QQuickItem *togetherNow = ub7Row(sc, a).value(QStringLiteral("groupName")).toString().contains(QLatin1String("UB7 new"))
                                      ? shown(QStringLiteral("Speakers.group.%1").arg(a)) : nullptr;
        check("the group is named by both speakers", togetherNow != nullptr);
    }
    d.grab(settings, QStringLiteral("ub7-speakers-byhand.png"));
    byHand = shown(QStringLiteral("Speakers.byHand"));
    ub7Toggle(byHand);
    settle();
    check("off again, the groups follow the names", byHand && !byHand->property("checked").toBool() && sc->groupsFollowNames()
                                                         && ub7Group(sc, b) != ub7Group(sc, a) && shown(QStringLiteral("Speakers.together.%1").arg(b)));
    sc->reload();
    sc->useSuggestion();
    sc->confirm();
    d.waitFor(5000, [&] { return sc->status() == QLatin1String("confirmed") && !sc->edited(); });
    settings->close();
    settle();

    // ---- Measure: the summary and this run's tick boxes -------------------------------------------
    const bool atMeasure = d.goTo(QStringLiteral("measure/measure"), 400);
    QQuickItem *root = d.mainWindow()->contentItem();
    std::printf("smoke: ub7: at measure %d, summary item %p shown %p, layout %s\n", atMeasure, (void *)d.item(QStringLiteral("MeasureSetup.speakers"), root),
                (void *)ub7Shown(root, QStringLiteral("MeasureSetup.speakers")), QJsonDocument(ub7Stored(d)).toJson(QJsonDocument::Compact).constData());
    QQuickItem *setup = ub7Shown(root, QStringLiteral("MeasureSetup.speakers"));
    QQuickItem *summaryText = ub7Shown(setup, QStringLiteral("SpeakersSummary.text"));
    const QString summary = sc->summary();
    std::printf("smoke: ub7: summary \"%s\", shown \"%s\"\n", qPrintable(summary), summaryText ? qPrintable(summaryText->property("text").toString()) : "-");
    check("Measure shows the layout in one line",
          summaryText && summaryText->property("text").toString() == summary && summary.endsWith(QStringLiteral(" · confirmed"))
              && summary.contains(QLatin1String("Out ")));
    QQuickItem *measurePage = ub7Shown(root, QStringLiteral("MeasureSetup.outputs"));
    check("the outputs as tick boxes", measurePage && ub7Shown(measurePage, QStringLiteral("MeasureSetup.output.%1").arg(a)));
    check("no role, speaker or group to edit on Measure",
          measurePage && ub7Labelled(measurePage->parentItem(), QStringLiteral("Way")) == 0 && ub7Labelled(measurePage->parentItem(), QStringLiteral("Speaker")) == 0
              && ub7Labelled(measurePage->parentItem(), QStringLiteral("Group")) == 0 && !ub7Shown(root, QStringLiteral("MeasureSetup.role.%1").arg(a)));
    const QJsonObject before = ub7Stored(d);
    bool onA = false;
    for (const QVariant &v : mc->setupOutputs())
        if (v.toMap().value(QStringLiteral("output")).toInt() == a) onA = v.toMap().value(QStringLiteral("on")).toBool();
    ub7Toggle(ub7Shown(measurePage, QStringLiteral("MeasureSetup.output.%1").arg(a)));
    settle();
    bool nowA = onA;
    for (const QVariant &v : mc->setupOutputs())
        if (v.toMap().value(QStringLiteral("output")).toInt() == a) nowA = v.toMap().value(QStringLiteral("on")).toBool();
    check("a tick is this run's: the layout keeps", nowA != onA && ub7Stored(d).value(QLatin1String("outputs")) == before.value(QLatin1String("outputs")));
    mc->setOutputOn(a, onA);
    d.grab(d.mainWindow(), QStringLiteral("ub7-measure.png"));
    settingsCtl->open(QStringLiteral("about"));
    settle();
    SmokeDriver::press(ub7Shown(setup, QStringLiteral("SpeakersSummary.edit")));
    settle();
    // Modern shows Settings as a page of the main window
    const bool speakersShown = ub7Shown(root, QStringLiteral("SpeakersPage")) || (settings && ub7Shown(settings->contentItem(), QStringLiteral("SpeakersPage")));
    std::printf("smoke: ub7: after Edit: page %s, shown %d\n", qPrintable(settingsCtl->currentPage()), speakersShown);
    check("Edit in Speakers… opens Settings › Speakers", settingsCtl->currentPage() == QLatin1String("speakers") && speakersShown);
    if (settings) settings->close();
    d.goTo(QStringLiteral("measure/measure"), 300);

    // An unconfirmed layout says so in the line
    QJsonObject kept = ub7Stored(d);
    ub7Keep(d, kept, false);
    sc->reload();
    d.waitFor(5000, [&] { return sc->status() != QLatin1String("confirmed"); });
    std::printf("smoke: ub7: unconfirmed: %s \"%s\"\n", qPrintable(sc->status()), qPrintable(sc->summary()));
    check("an unconfirmed layout: the line says so",
          d.waitFor(5000, [&] { return sc->status() != QLatin1String("confirmed"); }) && sc->summary().endsWith(QStringLiteral(" · not confirmed")));
    ub7Keep(d, kept, true);
    sc->reload();
    d.waitFor(5000, [&] { return sc->status() == QLatin1String("confirmed"); });

    // ---- Auto-tune: the same line, the outputs to tune ------------------------------------------
    d.goTo(QStringLiteral("measure/tune"), 400);
    QQuickItem *tune = ub7Shown(root, QStringLiteral("MeasureTune.page"));
    QQuickItem *tuneSummary = ub7Shown(ub7Shown(tune, QStringLiteral("MeasureTune.speakers")), QStringLiteral("SpeakersSummary.text"));
    check("Auto-tune shows the same line", tuneSummary && tuneSummary->property("text").toString() == sc->summary());
    check("and no role, speaker or group to edit there",
          tune && ub7Labelled(tune, QStringLiteral("Way")) == 0 && ub7Labelled(tune, QStringLiteral("Speaker")) == 0 && ub7Labelled(tune, QStringLiteral("Group")) == 0);
    bool tuneTicks = false;
    for (const QVariant &v : tc->layout()) tuneTicks = tuneTicks || ub7Shown(tune, QStringLiteral("MeasureTune.output.%1").arg(v.toMap().value(QStringLiteral("output")).toInt()));
    check("the outputs to tune as tick boxes", tc->layout().isEmpty() || tuneTicks);
    d.grab(d.mainWindow(), QStringLiteral("ub7-tune.png"));

    // ---- Left as found ---------------------------------------------------------------------------
    if (!found.value(QLatin1String("value")).isNull() && found.contains(QLatin1String("outputs"))) ub7Keep(d, found, foundConfirmed);
    sc->reload();
    d.waitFor(5000, [&] { return sc->loaded(); });
    d.goTo(QStringLiteral("overview/overview"), 200);
    if (!wasModern) d.switchLayout("classic");
    check("left as found", device->connected());
}

} // namespace

// After ua4-firstrun (the same order, by name)
const SmokeDriver::Section ub7("ub7-speakers", 205, &ub7Section);
