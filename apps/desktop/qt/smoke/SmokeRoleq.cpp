// -- WP ROLEQ -- The speaker layout on the demo DSPi: the Getting Started
// step shows the proposal and Confirm keeps it; Settings › Speakers edits and
// confirms; the Measure tables are pre-filled from the layout and write an
// edit back without confirming it; the banner while it is not confirmed (an
// imported layout: the core's import path, speaker_layout_set unconfirmed);
// the measure.layout_unconfirmed refusal in words through its prefix; and
// Android's getting_started.json without the step.
//
// The layouts live in this run's <AppData>/speaker-layouts (the run has a HOME
// of its own). The section ends with the proposal confirmed, so the Measure
// sections after it measure the outputs and roles they did before.
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "MeasureController.h"
#include "OnboardingController.h"
#include "Registrar.h"
#include "Shell.h"
#include "SpeakersController.h"
#include "TuneController.h"

#include <QDirIterator>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QStandardPaths>
#include <functional>

namespace {

using Job = std::function<QByteArray(FfiCore *)>;

// A core call on the worker, waited for
QJsonObject onWorker(SmokeDriver &d, Job job) {
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

QByteArray fill(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(1 << 16, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

QJsonObject storedLayout(SmokeDriver &d) {
    return onWorker(d, [](FfiCore *c) { return fill([c](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); }); });
}

QJsonObject proposal(SmokeDriver &d) {
    return onWorker(d, [](FfiCore *c) { return fill([c](char *b, uint32_t l) { return dspi_speaker_layout_propose(c, b, l); }); });
}

// What a configuration import does with a document's layout: kept, unconfirmed
// (review RA F1), not through the app
QJsonObject keepUnconfirmed(SmokeDriver &d, QJsonObject layout) {
    layout.remove(QStringLiteral("ok"));
    layout.insert(QStringLiteral("confirmedByUser"), false);
    const QByteArray text = QJsonDocument(layout).toJson(QJsonDocument::Compact);
    return onWorker(d, [text](FfiCore *c) {
        return fill([c, &text](char *b, uint32_t l) { return dspi_speaker_layout_set(c, text.constData(), b, l); });
    });
}

// -- WP ADETQ -- a layout's rows without whose word they are (ADET0's `source`:
// a proposal's rows are suggestions, the confirmed ones the user's)
QJsonArray rowsOf(const QJsonObject &layout) {
    QJsonArray out;
    for (const QJsonValue &v : layout.value(QLatin1String("outputs")).toArray()) {
        QJsonObject o = v.toObject();
        o.remove(QStringLiteral("source"));
        out.append(o);
    }
    return out;
}

QString roleIn(const QJsonObject &layout, int output) {
    for (const QJsonValue &v : layout.value(QLatin1String("outputs")).toArray())
        if (v.toObject().value(QLatin1String("output")).toInt() == output) return v.toObject().value(QLatin1String("role")).toString();
    return {};
}

QString rowRole(MeasureController *mc, int output) {
    for (const QVariant &v : mc->setupOutputs())
        if (v.toMap().value(QStringLiteral("output")).toInt() == output) return v.toMap().value(QStringLiteral("role")).toString();
    return {};
}

QVariantMap tuneRow(TuneController *tc, int output) {
    for (const QVariant &v : tc->layout())
        if (v.toMap().value(QStringLiteral("output")).toInt() == output) return v.toMap();
    return {};
}

void press(QObject *button) {
    if (button) QMetaObject::invokeMethod(button, "clicked");
}

QVariantList stageIds(const QVariantMap &words) {
    QVariantList ids;
    for (const QVariant &s : words.value(QStringLiteral("stages")).toList()) ids << s.toMap().value(QStringLiteral("id"));
    return ids;
}

// The item under `scope` that has property `name` (a tab's root from one of its children)
QQuickItem *upTo(QQuickItem *from, const char *name) {
    for (QQuickItem *i = from; i; i = i->parentItem())
        if (i->metaObject()->indexOfProperty(name) >= 0) return i;
    return nullptr;
}

QByteArray readCoreWizard() {
    return fill([](char *b, uint32_t l) { return dspi_onboarding_getting_started_json(b, l); });
}

bool isImported(SmokeDriver &d, SpeakersController *sc) {
    return d.waitFor(3000, [&] { return sc->status() == QLatin1String("imported"); });
}

void roleqSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    if (!device->connected() || !device->demo()) device->openDemo();
    d.check(QStringLiteral("roleq: the demo DSPi is open"), d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    auto *sc = Registrar::instance<SpeakersController>();
    auto *ob = Registrar::instance<OnboardingController>();
    auto *mc = Registrar::instance<MeasureController>();
    d.check(QStringLiteral("roleq: the controllers exist"), sc && ob && mc);
    if (!sc || !ob || !mc) return;

    // ---- Android's wizard stays as it is ----------------------------------------------
    const QJsonObject core = QJsonDocument::fromJson(readCoreWizard()).object();
    const QVariantList coreIds = stageIds(core.toVariantMap());
    const QVariantList appIds = stageIds(ob->gettingStarted());
    d.check(QStringLiteral("roleq: the core's getting_started.json (Android's) has no Speakers step"),
            !coreIds.isEmpty() && !coreIds.contains(QStringLiteral("speakers")));
    d.check(QStringLiteral("roleq: this app's wizard has it, before Done (speakers.json)"),
            appIds.size() == coreIds.size() + 1 && appIds.indexOf(QStringLiteral("speakers")) == appIds.indexOf(QStringLiteral("done")) - 1);

    // ---- No layout yet: the proposal --------------------------------------------------
    sc->reload();
    d.check(QStringLiteral("roleq: the layout is read"), d.waitFor(5000, [&] { return sc->loaded(); }));
    const QJsonObject proposed = proposal(d);
    const QJsonArray proposedOuts = proposed.value(QLatin1String("outputs")).toArray();
    d.check(QStringLiteral("roleq: a fresh core has no layout, only the suggestion"),
            sc->status() == QLatin1String("none") && storedLayout(d).value(QLatin1String("value")).isNull() && !proposedOuts.isEmpty());
    bool rowsAreProposal = sc->rows().size() == device->numOutputs();
    for (const QVariant &v : sc->rows()) {
        const QVariantMap r = v.toMap();
        const QString want = roleIn(proposed, r.value(QStringLiteral("output")).toInt());
        rowsAreProposal = rowsAreProposal && r.value(QStringLiteral("included")).toBool() == !want.isEmpty()
                          && (want.isEmpty() || r.value(QStringLiteral("role")).toString() == want);
    }
    d.check(QStringLiteral("roleq: the draft is the proposal, one row an output"), rowsAreProposal);

    // ---- The Getting Started step --------------------------------------------------------
    d.app().shell->trigger(QStringLiteral("help.gettingStarted"));
    d.spin(300);
    QObject *loader = d.object(QStringLiteral("MainWindow.wizard"));
    auto *wizard = loader ? loader->property("item").value<QQuickItem *>() : nullptr;
    d.check(QStringLiteral("roleq: Help > Getting Started shows the wizard"), ob->wizardShown() && wizard);
    if (wizard) {
        wizard->setProperty("current", appIds.indexOf(QStringLiteral("speakers")));
        d.spin(300);
        QQuickItem *step = d.item(QStringLiteral("SpeakersStep"), wizard);
        QQuickItem *title = d.item(QStringLiteral("GettingStarted.title"), wizard);
        d.check(QStringLiteral("roleq: the Speakers step asks which output drives what"),
                step && step->isVisible() && title && title->property("text").toString() == QLatin1String("Which output drives what?"));
        int rows = 0;
        for (int o = 0; o < device->numOutputs(); ++o)
            if (QQuickItem *r = d.item(QStringLiteral("Speakers.row.%1").arg(o), wizard); r && r->isVisible()) ++rows;
        QQuickItem *role0 = d.item(QStringLiteral("Speakers.role.%1").arg(proposedOuts.first().toObject().value(QLatin1String("output")).toInt()), wizard);
        d.check(QStringLiteral("roleq: the step shows every output, pre-filled from the proposal"),
                rows == device->numOutputs() && role0 && role0->property("currentText").toString().length() > 0);
        QQuickItem *state = d.item(QStringLiteral("SpeakersStep.state"), wizard);
        d.check(QStringLiteral("roleq: it says the roles are suggested"),
                state && state->property("text").toString().startsWith(QLatin1String("Suggested from the filters")));
        d.grab(d.mainWindow(), QStringLiteral("roleq-step.png"));
        press(d.item(QStringLiteral("SpeakersStep.confirm"), wizard));
        d.check(QStringLiteral("roleq: Confirm keeps the proposal as the confirmed layout"),
                d.waitFor(5000, [&] { return sc->status() == QLatin1String("confirmed"); }));
        const QJsonObject kept = storedLayout(d);
        d.check(QStringLiteral("roleq: the core has it, confirmed, with the proposal's outputs"),
                kept.value(QLatin1String("confirmedByUser")).toBool() && rowsOf(kept) == rowsOf(QJsonObject{ { QStringLiteral("outputs"), proposedOuts } }));   // -- WP ADETQ --
        // -- WP INT2 -- in the folder main.cpp names (MDQT1)
        QDirIterator files(QStandardPaths::writableLocation(QStandardPaths::AppDataLocation) + QStringLiteral("/speaker-layouts"),
                           { QStringLiteral("speaker_layout.json") }, QDir::Files, QDirIterator::Subdirectories);
        QFile file(files.hasNext() ? files.next() : QString());
        const QJsonObject onDisk = file.open(QIODevice::ReadOnly) ? QJsonDocument::fromJson(file.readAll()).object() : QJsonObject();
        d.check(QStringLiteral("roleq: it is kept in the app's speaker-layouts folder"),
                onDisk.value(QLatin1String("confirmedByUser")).toBool()
                    && onDisk.value(QLatin1String("updatedMs")) == kept.value(QLatin1String("updatedMs")));
        d.spin(100);
        d.check(QStringLiteral("roleq: the step says it is confirmed"),
                state && state->property("text").toString().startsWith(QLatin1String("Confirmed.")));
        d.grab(d.mainWindow(), QStringLiteral("roleq-step-confirmed.png"));
    }
    ob->finishSetup();
    d.spin(200);
    d.check(QStringLiteral("roleq: the wizard closes"), !ob->wizardShown());

    // ---- Settings › Speakers ----------------------------------------------------------
    const int out = proposedOuts.first().toObject().value(QLatin1String("output")).toInt();
    const QString was = roleIn(proposed, out);
    const QString other = was == QLatin1String("mid") ? QStringLiteral("low") : QStringLiteral("mid");
    sc->openSettings();
    auto *settings = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("edit.settings")));
    d.check(QStringLiteral("roleq: Settings › Speakers opens"),
            settings && d.waitFor(3000, [&] { return settings->isVisible() && d.item(QStringLiteral("SpeakersPage"), settings->contentItem()); }));
    QQuickItem *page = settings ? d.item(QStringLiteral("SpeakersPage"), settings->contentItem()) : nullptr;
    QQuickItem *pageBanner = page ? d.item(QStringLiteral("SpeakersPage.banner"), page) : nullptr;
    d.check(QStringLiteral("roleq: a confirmed layout shows no banner"), pageBanner && !pageBanner->isVisible());
    sc->setRole(out, other);
    d.spin(100);
    QQuickItem *pageState = page ? d.item(QStringLiteral("SpeakersPage.state"), page) : nullptr;
    d.check(QStringLiteral("roleq: an edit is a draft until Confirm"),
            sc->edited() && pageState && pageState->property("text").toString().startsWith(QLatin1String("Changed"))
                && roleIn(storedLayout(d), out) == was);
    press(page ? d.item(QStringLiteral("SpeakersPage.confirm"), page) : nullptr);
    d.check(QStringLiteral("roleq: the page's Confirm keeps the edit, confirmed"),
            d.waitFor(5000, [&] { return !sc->edited(); }) && roleIn(storedLayout(d), out) == other
                && storedLayout(d).value(QLatin1String("confirmedByUser")).toBool());
    if (settings) d.grab(settings, QStringLiteral("roleq-settings.png"));

    // ---- Measure: pre-filled, an edit written back ----------------------------------------
    auto *mw = qobject_cast<QQuickWindow *>(d.app().shell->openWindow(QStringLiteral("tools.measure")));
    d.check(QStringLiteral("roleq: the Measure window opens"), mw && d.waitFor(2000, [&] { return mw->isVisible(); }));
    // The Measure tab in front (its banner is what shows)
    if (QQuickItem *tabs = mw ? d.item(QStringLiteral("Measure.tabs"), mw->contentItem()) : nullptr) tabs->setProperty("currentIndex", 0);
    d.spin(150);
    d.check(QStringLiteral("roleq: the Measure rows are pre-filled from the layout"),
            d.waitFor(3000, [&] { return rowRole(mc, out) == other; }));
    // -- WP RCFIX -- (review RC, RC2) and Auto-tune's rows follow a Confirm, no Reload
    auto *tc = Registrar::instance<TuneController>();
    const auto tuneRole = [&] { return tuneRow(tc, out).value(QStringLiteral("role")).toString(); };
    d.check(QStringLiteral("roleq: the Auto-tune rows are pre-filled from the layout"),
            tc && d.waitFor(5000, [&] { return tuneRole() == other; }));
    if (tc) {
        tc->setOutputOn(out, !tuneRow(tc, out).value(QStringLiteral("on")).toBool());
        const bool on = tuneRow(tc, out).value(QStringLiteral("on")).toBool();
        QString asTold;
        const QMetaObject::Connection told = QObject::connect(sc, &SpeakersController::storedChanged, sc, [&] { asTold = tuneRole(); },
                                                              Qt::QueuedConnection);
        sc->setRole(out, QStringLiteral("high"));
        sc->confirm();
        d.check(QStringLiteral("roleq: a Confirm reaches the Auto-tune rows at once, their chosen outputs stay"),
                d.waitFor(5000, [&] { return !asTold.isEmpty(); }) && asTold == QLatin1String("high")
                    && tuneRow(tc, out).value(QStringLiteral("on")).toBool() == on);
        QObject::disconnect(told);
        tc->setOutputOn(out, !on);
        sc->setRole(out, other);
        sc->confirm();
        d.check(QStringLiteral("roleq: and the next one"),
                d.waitFor(5000, [&] { return tuneRole() == other && roleIn(storedLayout(d), out) == other; }) && rowRole(mc, out) == other);
    }
    QQuickItem *setupBanner = mw ? d.item(QStringLiteral("MeasureSetup.speakersBanner"), mw->contentItem()) : nullptr;
    d.check(QStringLiteral("roleq: no banner on Measure while the layout is confirmed"), setupBanner && !setupBanner->isVisible());
    mc->setOutputRole(out, was);
    d.check(QStringLiteral("roleq: a Measure edit is written back to the layout"),
            d.waitFor(5000, [&] { return roleIn(storedLayout(d), out) == was; }) && sc->said() == QLatin1String("Saved to your speaker layout."));
    d.check(QStringLiteral("roleq: and it stays confirmed (only Confirm confirms)"),
            storedLayout(d).value(QLatin1String("confirmedByUser")).toBool() && sc->status() == QLatin1String("confirmed"));
    QQuickItem *liveRole = mw ? d.item(QStringLiteral("MeasureLive.role"), mw->contentItem()) : nullptr;
    QQuickItem *liveTab = upTo(liveRole, "placementChoice");
    if (liveTab) {
        liveTab->setProperty("output", out);
        d.spin(100);
    }
    d.check(QStringLiteral("roleq: the Live tab takes the output's role from the layout"),
            liveTab && liveTab->property("role").toString() == was);
    if (liveTab) std::printf("smoke: roleq: live output %d role %s (layout %s)\n", liveTab->property("output").toInt(),
                             qPrintable(liveTab->property("role").toString()), qPrintable(was));

    // ---- An imported layout: unconfirmed, flagged ---------------------------------------
    QJsonObject imported = storedLayout(d);
    // -- WP INT2 -- an import this console has not read yet, then a Measure edit
    keepUnconfirmed(d, imported);
    sc->writeBack(out, was);
    d.check(QStringLiteral("roleq: an edit right after an import does not take the import for this console's own"), isImported(d, sc));
    imported = keepUnconfirmed(d, imported);
    sc->reload();
    d.check(QStringLiteral("roleq: an unconfirmed layout this console did not write reads as imported"),
            isImported(d, sc) && !storedLayout(d).value(QLatin1String("confirmedByUser")).toBool());
    d.spin(100);
    if (setupBanner)
        std::printf("smoke: roleq: banner visible %d kind %s text %s\n", setupBanner->isVisible(),
                    qPrintable(setupBanner->property("kind").toString()), qPrintable(setupBanner->property("text").toString()));
    d.check(QStringLiteral("roleq: Measure shows the imported banner, pointing to Settings › Speakers"),
            setupBanner && setupBanner->isVisible() && setupBanner->property("kind").toString() == QLatin1String("warning")
                && setupBanner->property("text").toString().startsWith(QLatin1String("Imported speaker layout"))
                && setupBanner->property("text").toString().contains(QString::fromUtf8("Settings › Speakers")));
    d.check(QStringLiteral("roleq: the Measure rows still come from it"), rowRole(mc, out) == was);
    if (mw) d.grab(mw, QStringLiteral("roleq-measure-imported.png"));
    mc->setOutputRole(out, other);
    d.check(QStringLiteral("roleq: a Measure edit of an imported layout keeps it imported and unconfirmed"),
            d.waitFor(5000, [&] { return roleIn(storedLayout(d), out) == other; }) && sc->status() == QLatin1String("imported")
                && !storedLayout(d).value(QLatin1String("confirmedByUser")).toBool());
    d.check(QStringLiteral("roleq: the Live tab follows a Setup edit of the layout"),
            liveTab && d.waitFor(2000, [&] { return liveTab->property("role").toString() == other; }));
    d.check(QStringLiteral("roleq: the page shows the imported banner too"),
            pageBanner && pageBanner->isVisible() && pageBanner->property("text").toString().startsWith(QLatin1String("Imported")));

    // ---- The refusal's words, by its prefix ----------------------------------------------
    const QByteArray setup = QJsonDocument(QJsonObject{ { QStringLiteral("kind"), QStringLiteral("walked") },
                                                        { QStringLiteral("outputs"), QJsonArray{} },
                                                        { QStringLiteral("placement"), QStringLiteral("listeningPosition") },
                                                        { QStringLiteral("sweepMs"), 3000 },
                                                        { QStringLiteral("passes"), 3 },
                                                        { QStringLiteral("rolesFromLayout"), true } })
                                 .toJson(QJsonDocument::Compact);
    const QJsonObject refused = onWorker(d, [setup](FfiCore *c) {
        return fill([c, &setup](char *b, uint32_t l) { return dspi_measure_preflight(c, setup.constData(), b, l); });
    });
    const QString words = MeasureController::failureText(refused);
    d.check(QStringLiteral("roleq: measuring from an unconfirmed layout is refused with the notice's title (prefix match)"),
            !refused.value(QLatin1String("ok")).toBool()
                && refused.value(QLatin1String("error")).toString().contains(QLatin1String("measure.layout_unconfirmed:"))
                && words.startsWith(QLatin1String("Speaker layout not confirmed: ")));
    std::printf("smoke: roleq: the refusal: noticeKey %s, in words \"%s\"\n",
                qPrintable(refused.value(QLatin1String("noticeKey")).toString()), qPrintable(words));

    // ---- Confirm clears it; the proposal confirmed again for the sections after ---------
    press(page ? d.item(QStringLiteral("SpeakersPage.suggestion"), page) : nullptr);
    d.spin(100);
    press(page ? d.item(QStringLiteral("SpeakersPage.confirm"), page) : nullptr);
    d.check(QStringLiteral("roleq: Confirm on the page clears the imported flag"),
            d.waitFor(5000, [&] { return sc->status() == QLatin1String("confirmed"); })
                && rowsOf(storedLayout(d)) == rowsOf(QJsonObject{ { QStringLiteral("outputs"), proposedOuts } }));   // -- WP ADETQ --
    d.spin(100);
    d.check(QStringLiteral("roleq: and the banners go"), setupBanner && !setupBanner->isVisible() && pageBanner && !pageBanner->isVisible());
    d.check(QStringLiteral("roleq: the Measure rows follow the confirmed layout"), d.waitFor(3000, [&] { return rowRole(mc, out) == was; }));
    if (settings) settings->close();
    if (mw) mw->close();
    d.spin(100);
}

const SmokeDriver::Section roleq("roleq-speakers", 205, &roleqSection);

} // namespace
