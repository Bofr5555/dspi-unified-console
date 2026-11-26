// -- WP UA2 -- (A2, QT-MEASUREBAR) Measure and Auto-tune with the actions in
// view, in Modern at 1600×1000 on the demo with its simulated microphone:
// - "ua2-measurebar": a refused Measure shows one row per problem (the volume
//   too high, an output muted), the volume's with "Lower to −12 dB", which
//   lowers it and the row goes, the one sentence no more; the run's step,
//   progress and Stop in the action bar; the core's default measurement name
//   as "<device> · 25 Nov 21:23"; "see Results" cleared once Results show; the
//   proposal's Apply… in the bar, the details folded; the summed check keeps
//   the page where it was; after Apply the countdown, Keep (primary) and
//   Revert (plain) in the bar, inside the window, on the Auto-tune tab and on
//   Results; Revert puts the demo back.
// Each state is grabbed in the dark and the light scheme into <out>/ua2/ (the
// report's pictures). The checks read the controllers through
// their properties, so the walk also runs on a tree without this package (the
// "before" pictures; its checks fail there).
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "MeasureController.h"
#include "Registrar.h"
#include "Shell.h"
#include "TuneController.h"

#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>
#include <QSignalSpy>
#include <algorithm>
#include <cmath>
#include <functional>
#include <utility>

namespace {

void press(QObject *button) {
    if (button) QMetaObject::invokeMethod(button, "clicked");
}

// Shown, of a size, and whole inside the window: seen without scrolling
bool inView(QQuickWindow *w, QQuickItem *item) {
    if (!w || !item || !item->isVisible() || item->width() <= 0 || item->height() <= 0) return false;
    const QRectF r = item->mapRectToScene(QRectF(0, 0, item->width(), item->height()));
    return r.left() >= 0 && r.top() >= 0 && r.right() <= w->width() && r.bottom() <= w->height();
}

// Where an item is in the window now (its top), to see that the page did not move
qreal sceneTop(QQuickItem *item) { return item ? item->mapToScene(QPointF(0, 0)).y() : -1e9; }

// The flickable of the Auto-tune tab's page
QQuickItem *tuneFlick(SmokeDriver &d, QQuickWindow *w) {
    QQuickItem *page = d.item(QStringLiteral("MeasureTune.page"), w->contentItem());
    return page ? page->property("contentItem").value<QQuickItem *>() : nullptr;
}

// The window in the dark and the light scheme, as <out>/ua2/<name>-<scheme>.png
void grabBoth(SmokeDriver &d, QQuickWindow *w, const QString &name) {
    Shell *shell = d.app().shell;
    const Qt::ColorScheme was = shell->colorSchemeOverride();
    for (const auto &[scheme, word] : { std::pair{ Qt::ColorScheme::Dark, "dark" }, std::pair{ Qt::ColorScheme::Light, "light" } }) {
        shell->setColorSchemeOverride(scheme);
        d.spin(250);
        d.grab(w, QStringLiteral("ua2/%1-%2.png").arg(name, QLatin1String(word)));
    }
    shell->setColorSchemeOverride(was);
    d.spin(150);
}

void writeVolume(SmokeDriver &d, float db) {
    d.app().device->write(QStringLiteral("ua2 volume"), [db](FfiCore *c) { return dspi_set_user_volume(c, db); });
    d.waitIdle();
}

void writeMute(SmokeDriver &d, int output, bool muted) {
    d.app().device->write(QStringLiteral("ua2 mute %1").arg(output), [output, muted](FfiCore *c) { return dspi_set_output_mute(c, uint8_t(output), muted); });
    d.waitIdle();
}

QByteArray fill(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(1 << 16, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

// A core call on the worker, waited for
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

QJsonObject setLayout(SmokeDriver &d, const QJsonObject &layout) {
    const QByteArray text = QJsonDocument(layout).toJson(QJsonDocument::Compact);
    return onWorker(d, [text](FfiCore *c) { return fill([c, &text](char *b, uint32_t n) { return dspi_speaker_layout_set(c, text.constData(), b, n); }); });
}

// Crossover band 1 of a wire channel
void writeBand(SmokeDriver &d, int channel, const FilterParams &p) {
    d.app().device->write(QStringLiteral("ua2 xover %1").arg(channel), [channel, p](FfiCore *c) { return dspi_set_crossover(c, uint8_t(channel), 0, p); });
    d.waitIdle();
}

void measureBarSection(SmokeDriver &d) {
    d.allowSectionTime(420);
    auto *mc = Registrar::instance<MeasureController>();
    auto *tc = Registrar::instance<TuneController>();
    DeviceModel *device = d.app().device;
    QQuickWindow *w = d.mainWindow();
    d.check("ua2: the controllers and the window", mc && tc && w);
    if (!mc || !tc || !w) return;
    if (!device->connected() || !device->demo() || device->remote()) device->openDemo();
    d.check("ua2: on the demo", d.waitFor(10000, [&] { return device->connected() && device->demo(); }) && d.waitIdle());
    if (!device->state() || !d.switchLayout("modern")) {
        d.check("ua2: in Modern", false);
        return;
    }
    QDir(d.outDir() + QStringLiteral("/ua2")).removeRecursively();   // this run's pictures only
    QDir().mkpath(d.outDir() + QStringLiteral("/ua2"));
    const float volumeWas = device->state()->user_volume_db;
    // A two-way left speaker, so the tune has a transition to check: Out 1 its
    // woofer (LR24 low-pass 2 kHz), Out 2 its tweeter (LR24 high-pass 2 kHz,
    // which the tweeter rule asks for), both fed by USB 1, in a confirmed
    // layout; all put back at the end
    auto *cc = Registrar::instance<ChannelController>();
    const int woofer = cc ? cc->outputChannel(0) : -1, tweeter = cc ? cc->outputChannel(1) : -1;
    const QJsonValue layoutWas = onWorker(d, [](FfiCore *c) { return fill([c](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); }); })
                                     .value(QLatin1String("value"));
    FilterParams wooferWas{}, tweeterWas{};
    float delayWas = 0;
    struct Route { bool on; float gain; bool invert; } r0Was{}, r1Was{};
    if (woofer >= 0 && tweeter >= 0) {
        wooferWas = device->state()->xover[woofer][0];
        tweeterWas = device->state()->xover[tweeter][0];
        FilterParams lp{};
        lp.filter_type = 34;   // LR4 (24 dB/octave) low-pass (dspi-proto xover.rs)
        lp.freq = 2000.0f;
        lp.q = 0.707f;
        FilterParams hp = lp;
        hp.filter_type = 35;   // LR4 high-pass
        writeBand(d, woofer, lp);
        writeBand(d, tweeter, hp);
        // Both ways fed by USB 1 (one programme), USB 2 off the tweeter
        r0Was = { device->state()->matrix_routing[0][1], device->state()->matrix_gain[0][1], device->state()->matrix_invert[0][1] };
        r1Was = { device->state()->matrix_routing[1][1], device->state()->matrix_gain[1][1], device->state()->matrix_invert[1][1] };
        device->write(QStringLiteral("ua2 route 0"), [](FfiCore *c) { return dspi_set_matrix_route(c, 0, 1, true, 0.0f, false); });
        device->write(QStringLiteral("ua2 route 1"), [](FfiCore *c) { return dspi_set_matrix_route(c, 1, 1, false, 0.0f, false); });
        d.waitIdle();
        // The tweeter late by 0.6 ms: the tune has an arrival to align
        delayWas = device->state()->output_delay_ms[1];
        device->write(QStringLiteral("ua2 delay"), [](FfiCore *c) { return dspi_set_output_delay(c, 1, 0.6f); });
        d.waitIdle();
        const auto way = [](int o, const char *role) {
            return QJsonObject{ { QStringLiteral("output"), o }, { QStringLiteral("role"), QLatin1String(role) },
                                { QStringLiteral("speaker"), QStringLiteral("L") }, { QStringLiteral("group"), 0 } };
        };
        setLayout(d, QJsonObject{ { QStringLiteral("outputs"), QJsonArray{ way(0, "low"), way(1, "high") } },
                                  { QStringLiteral("confirmedByUser"), true } });
        device->refresh();
        d.waitIdle();
    }
    d.check("ua2: a two-way speaker on Out 1 and Out 2", woofer >= 0 && tweeter >= 0);
    const QSize original = w->size();
    w->resize(QSize(1600, 1000));
    d.goTo(QStringLiteral("measure/measure"), 400);
    const auto item = [&](const char *name) { return d.item(QString::fromLatin1(name), w->contentItem()); };

    // The setup: the layout's first speaker, short sweeps, no name (the core's default)
    mc->fillFromLayout();
    d.waitIdle();
    d.waitFor(5000, [&] { return !mc->setupOutputs().isEmpty(); });
    // The two ways, whatever the setup held before (the layout's answer comes
    // on the loop): Out 1 the woofer, Out 2 the tweeter of speaker L
    const QList<int> on{ 0, 1 };
    for (const QVariant &v : mc->setupOutputs()) {
        const int o = v.toMap().value(QStringLiteral("output")).toInt();
        mc->setOutputOn(o, on.contains(o));
    }
    mc->setOutputRole(0, QStringLiteral("low"));
    mc->setOutputRole(1, QStringLiteral("high"));
    for (int o : on) {
        mc->setOutputSpeaker(o, QStringLiteral("L"));
        mc->setOutputGroup(o, 0);
    }
    mc->setPlacement(QStringLiteral("listeningPosition"));
    mc->setSweepMs(1000);
    mc->setPasses(3);
    mc->setRunName(QString());
    d.spin(150);
    d.check("ua2: a setup to measure", !on.isEmpty() && mc->setupProblem().isEmpty());
    if (on.isEmpty()) return;

    // ---- A refused Measure: one row per problem, the volume's with its fix ----
    writeVolume(d, -2.0f);
    writeMute(d, on.first(), true);
    mc->clearError();
    press(item("MeasureSetup.start"));
    const auto refusals = [&] { return mc->property("refusals").toList(); };
    const bool refused = d.waitFor(8000, [&] { return !mc->error().isEmpty(); }) && d.waitFor(3000, [&] { return refusals().size() >= 2; });
    QStringList keys;
    int volumeRow = -1;
    for (int i = 0; i < refusals().size(); ++i) {
        keys << refusals().at(i).toMap().value(QStringLiteral("key")).toString();
        if (keys.last() == QLatin1String("measure.volume_high")) volumeRow = i;
    }
    std::printf("smoke: ua2: refused: %s | %s\n", qPrintable(keys.join(QLatin1Char(','))), qPrintable(mc->error()));
    d.check(QStringLiteral("ua2: a refused Measure names each problem on its own (%1)").arg(keys.join(QStringLiteral(", "))),
            refused && keys.contains(QLatin1String("measure.volume_high")) && keys.contains(QLatin1String("measure.output_muted")) &&
                !mc->running());
    QQuickItem *row0 = item("Measure.refusal.0");
    QQuickItem *row1 = item("Measure.refusal.1");
    QQuickItem *sentence = item("Measure.error");
    d.check("ua2: one row each, in view, the joined sentence not shown",
            inView(w, row0) && inView(w, row1) && !(sentence && sentence->isVisible()));
    QQuickItem *fix = volumeRow >= 0 ? item(qPrintable(QStringLiteral("Measure.refusal.%1.notice.action").arg(volumeRow))) : nullptr;
    QQuickItem *mutedFix = item(qPrintable(QStringLiteral("Measure.refusal.%1.notice.action").arg(volumeRow == 0 ? 1 : 0)));
    d.check(QStringLiteral("ua2: the volume's row offers its fix (%1); the muted output's none").arg(fix ? fix->property("text").toString() : QString()),
            fix && fix->isVisible() && fix->property("text").toString() == QStringLiteral("Lower to −12 dB") && !(mutedFix && mutedFix->isVisible()));
    grabBoth(d, w, QStringLiteral("1-refused"));
    press(fix);
    QQuickItem *volumeItem = volumeRow >= 0 ? item(qPrintable(QStringLiteral("Measure.refusal.%1").arg(volumeRow))) : nullptr;
    d.check("ua2: Lower to −12 dB lowers the volume, and its row goes",
            fix && d.waitFor(5000, [&] { return d.waitIdle(500) && device->state()->user_volume_db <= -12.0f && device->state()->user_volume_db > -12.5f; }) &&
                volumeItem && d.waitFor(2000, [&] { return !volumeItem->isVisible(); }));
    writeMute(d, on.first(), false);
    mc->clearError();
    writeVolume(d, -20.0f);

    // ---- The run: its step, progress and Stop in the bar ----
    press(item("MeasureSetup.start"));
    const bool started = d.waitFor(8000, [&] { return mc->measuring(); });
    bool barShown = false, progressShown = false;
    if (started) d.waitFor(3000, [&] {
        barShown = barShown || (inView(w, item("Measure.stop")) && inView(w, item("Measure.runningText")));
        progressShown = progressShown || inView(w, item("Measure.progress"));
        return (barShown && progressShown) || !mc->measuring();
    });
    if (started && mc->measuring()) grabBoth(d, w, QStringLiteral("2-measuring"));
    const bool ended = started && d.waitFor(110000, [&] { return !mc->measuring() && !mc->running(); });
    d.check("ua2: while it measures, the bar shows the step, the progress and Stop in view", started && barShown && progressShown);
    d.check("ua2: the measurement completes", ended && d.waitFor(5000, [&] { return mc->hasResult() && !mc->running(); }));

    // The core's default name as the device and the time; "see Results" goes once Results show
    static const QRegularExpression human(QStringLiteral("^.+ · \\d{1,2} \\S+ \\d\\d:\\d\\d$"));
    d.check(QStringLiteral("ua2: the measurement is named for people (%1)").arg(mc->resultName()),
            human.match(mc->resultName()).hasMatch() && !mc->resultName().startsWith(QLatin1String("Measurement 20")));
    const bool saidComplete = mc->said().contains(QLatin1String("see Results"));
    d.goTo(QStringLiteral("measure/results"), 300);
    d.check("ua2: \"see Results\" is said, and cleared once Results show", saidComplete && mc->said().isEmpty());
    grabBoth(d, w, QStringLiteral("3-results"));

    // ---- The proposal: its actions in the bar, the details folded ----
    d.goTo(QStringLiteral("measure/tune"), 300);
    QSignalSpy steps(tc, &TuneController::stepDone);
    const auto step = [&](const QString &name, int ms) {
        return d.waitFor(ms, [&] {
            for (const QList<QVariant> &a : steps)
                if (a.value(0).toString() == name) return true;
            return false;
        });
    };
    // The tune's own choices (an earlier section may have left one on)
    const bool keptWas[4] = { tc->keepCrossover(), tc->keepDelays(), tc->keepLevels(), tc->keepEq() };
    tc->setKeepCrossover(false);
    tc->setKeepDelays(false);
    tc->setKeepLevels(false);
    tc->setKeepEq(false);
    tc->reloadLayout();
    d.waitIdle();
    d.spin(300);
    press(item("MeasureTune.propose"));
    const bool proposed = step(QStringLiteral("propose"), 30000) && tc->hasProposal();
    d.spin(300);
    QQuickItem *apply = item("MeasureTune.apply");
    QQuickItem *details = item("MeasureTune.details");
    d.check(QStringLiteral("ua2: the proposal's Apply… is in the bar, in view (%1)").arg(apply ? apply->property("text").toString() : QString()),
            proposed && inView(w, apply) && apply->property("text").toString() == QStringLiteral("Apply…"));
    const int detailCount = int(tc->held().size() + tc->withdrawn().size() + tc->notes().size());
    d.check(QStringLiteral("ua2: the proposal's %1 detail line(s) are folded").arg(detailCount),
            proposed && (detailCount == 0 || (details && details->isVisible() && !details->property("expanded").toBool())));
    {
        QStringList rows;
        for (const QVariant &v : tc->layout()) {
            const QVariantMap m = v.toMap();
            rows << QStringLiteral("%1:%2:%3:%4:%5").arg(m.value(QStringLiteral("output")).toInt()).arg(m.value(QStringLiteral("on")).toBool())
                        .arg(m.value(QStringLiteral("role")).toString(), m.value(QStringLiteral("speaker")).toString()).arg(m.value(QStringLiteral("group")).toInt());
        }
        QStringList measured;
        for (const QVariant &v : mc->resultOutputs()) measured << v.toMap().value(QStringLiteral("label")).toString();
        std::printf("smoke: ua2: tune layout %s; measured %s; keep %d%d%d%d\n", qPrintable(rows.join(QLatin1Char(' '))), qPrintable(measured.join(QStringLiteral(", "))), tc->keepCrossover(), tc->keepDelays(), tc->keepLevels(), tc->keepEq());
    }
    std::printf("smoke: ua2: proposal: %d write(s), timing %s; withdrawn: %s; notes: %s\n", int(tc->writes().size()),
                tc->timingReliable() ? "reliable" : "unreliable", qPrintable(tc->withdrawn().join(QStringLiteral(" | "))),
                qPrintable(tc->notes().join(QStringLiteral(" | "))));
    grabBoth(d, w, QStringLiteral("4-proposal"));
    if (!proposed || tc->writes().isEmpty()) {
        d.notRun(QStringLiteral("ua2: the trial"), QStringLiteral("no proposal with a change (%1)").arg(tc->error()));
    } else {
        // The details unfolded (the page scrolls then); the summed check keeps
        // the page where the reader left it
        if (details) details->setProperty("expanded", true);
        d.spin(300);
        if (tc->needsSumCheck()) {
            QQuickItem *flick = tuneFlick(d, w);
            const qreal room = flick ? flick->property("contentHeight").toReal() - flick->height() : 0;
            if (flick && room > 40) {
                const qreal at = std::min<qreal>(room, 160);
                flick->setProperty("contentY", at);
                d.spin(100);
                const qreal graphAt = sceneTop(item("MeasureTune.graph"));
                steps.clear();
                press(item("MeasureTune.sumCheckRun"));
                bool checkBar = false;
                const bool ran = d.waitFor(110000, [&] {
                    checkBar = checkBar || (tc->running() && inView(w, item("Measure.progress")) && inView(w, item("Measure.stop")));
                    for (const QList<QVariant> &a : steps)
                        if (a.value(0).toString() == QLatin1String("sumCheck")) return true;
                    return false;
                }) && d.waitFor(5000, [&] { return !mc->running(); });
                d.spin(400);
                const qreal now = sceneTop(item("MeasureTune.graph"));
                d.check(QStringLiteral("ua2: the summed check runs in the bar and keeps the page where it was (the graph at %1 → %2)").arg(graphAt).arg(now),
                        ran && checkBar && std::abs(now - graphAt) < 1.0);
            } else {
                d.notRun(QStringLiteral("ua2: the page kept across the summed check"), QStringLiteral("the page does not scroll"));
                tc->sumCheckRun();
                step(QStringLiteral("sumCheck"), 110000);
                d.waitFor(5000, [&] { return !mc->running(); });
            }
        } else {
            d.notRun(QStringLiteral("ua2: the summed check in the bar"), QStringLiteral("no timing change proposed (%1)").arg(tc->sumCheckText()));
        }
        // Apply… asks, then the trial: the countdown, Keep primary, Revert plain,
        // in view on every tab; the page stays where the reader left it
        QQuickItem *flick = tuneFlick(d, w);
        const qreal room = flick ? flick->property("contentHeight").toReal() - flick->height() : 0;
        const qreal at = std::min<qreal>(std::max<qreal>(room, 0), 160);
        if (flick && room > 40) flick->setProperty("contentY", at);
        d.spin(150);
        const qreal graphWas = sceneTop(item("MeasureTune.graph"));
        steps.clear();
        apply = item("MeasureTune.apply");
        press(apply);
        QQuickItem *bar = apply;
        while (bar && bar->objectName() != QLatin1String("Measure.actionBar")) bar = bar->parentItem();
        QObject *question = bar ? bar->findChild<QObject *>(QStringLiteral("MeasureTune.applyQuestion")) : nullptr;
        d.check("ua2: Apply… asks first", question && d.waitFor(2000, [&] { return question->property("opened").toBool(); }));
        if (question) QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
        else tc->apply(0);
        const bool applied = step(QStringLiteral("apply"), 15000) && d.waitIdle() && d.waitFor(3000, [&] { return tc->applied(); });
        d.spin(400);
        if (flick && room > 40) {
            const qreal now = sceneTop(item("MeasureTune.graph"));
            d.check(QStringLiteral("ua2: Apply keeps the page where it was (the proposal's graph at %1 → %2)").arg(graphWas).arg(now),
                    std::abs(now - graphWas) < 1.0);
        } else {
            d.notRun(QStringLiteral("ua2: the page kept across Apply"), QStringLiteral("the page does not scroll"));
        }
        QQuickItem *keep = item("MeasureTune.keep");
        QQuickItem *revert = item("MeasureTune.revert");
        QQuickItem *text = item("Measure.runningText");
        const QString words = text ? text->property("text").toString() : QString();
        d.check(QStringLiteral("ua2: the trial in the bar: %1").arg(words),
                applied && words.contains(QStringLiteral("goes back by itself in ")) && QRegularExpression(QStringLiteral("\\d+:\\d\\d$")).match(words).hasMatch());
        d.check("ua2: Keep (primary) and Revert (plain) in view on the Auto-tune tab",
                inView(w, keep) && inView(w, revert) && keep->property("primary").toBool() && !revert->property("destructive").toBool() &&
                    !revert->property("primary").toBool());
        grabBoth(d, w, QStringLiteral("5-trial"));
        d.goTo(QStringLiteral("measure/results"), 300);
        keep = item("MeasureTune.keep");
        d.check("ua2: and on Results", tc->applied() && inView(w, keep));
        grabBoth(d, w, QStringLiteral("6-trial-results"));
        d.goTo(QStringLiteral("measure/tune"), 300);
        steps.clear();
        press(item("MeasureTune.revert"));
        d.check("ua2: Revert from the bar puts the demo back",
                step(QStringLiteral("revert"), 15000) && d.waitIdle() && d.waitFor(5000, [&] { return tc->stage() == QLatin1String("reverted"); }));
        grabBoth(d, w, QStringLiteral("7-reverted"));
    }

    tc->stop();
    tc->setKeepCrossover(keptWas[0]);
    tc->setKeepDelays(keptWas[1]);
    tc->setKeepLevels(keptWas[2]);
    tc->setKeepEq(keptWas[3]);
    mc->clearError();
    writeVolume(d, volumeWas);
    if (woofer >= 0 && tweeter >= 0) {
        writeBand(d, woofer, wooferWas);
        writeBand(d, tweeter, tweeterWas);
        device->write(QStringLiteral("ua2 delay"), [delayWas](FfiCore *c) { return dspi_set_output_delay(c, 1, delayWas); });
        device->write(QStringLiteral("ua2 route 0"), [r0Was](FfiCore *c) { return dspi_set_matrix_route(c, 0, 1, r0Was.on, r0Was.gain, r0Was.invert); });
        device->write(QStringLiteral("ua2 route 1"), [r1Was](FfiCore *c) { return dspi_set_matrix_route(c, 1, 1, r1Was.on, r1Was.gain, r1Was.invert); });
        d.waitIdle();
    }
    if (layoutWas.isObject()) setLayout(d, layoutWas.toObject());
    device->refresh();
    d.waitIdle();
    w->resize(original);
    d.switchLayout("classic");
}

}   // namespace

static const SmokeDriver::Section measureBar("ua2-measurebar", 995, &measureBarSection);
