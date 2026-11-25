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
// DSPI_UA2_SHOTS=1 grabs each state in the dark and the light scheme into
// <out>/ua2/ (the report's pictures). The checks read the controllers through
// their properties, so the walk also runs on a tree without this package (the
// "before" pictures; its checks fail there).
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "MeasureController.h"
#include "Registrar.h"
#include "Shell.h"
#include "TuneController.h"

#include <QDir>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>
#include <QSignalSpy>
#include <algorithm>
#include <cmath>
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

// The flickable of the Auto-tune tab's page
QQuickItem *tuneFlick(SmokeDriver &d, QQuickWindow *w) {
    QQuickItem *page = d.item(QStringLiteral("MeasureTune.page"), w->contentItem());
    return page ? page->property("contentItem").value<QQuickItem *>() : nullptr;
}

bool shots() { return qEnvironmentVariableIsSet("DSPI_UA2_SHOTS"); }

// The window in the dark and the light scheme, as <out>/ua2/<name>-<scheme>.png
void grabBoth(SmokeDriver &d, QQuickWindow *w, const QString &name) {
    if (!shots()) return;
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
    QDir().mkpath(d.outDir() + QStringLiteral("/ua2"));
    const float volumeWas = device->state()->user_volume_db;
    const QSize original = w->size();
    w->resize(QSize(1600, 1000));
    d.goTo(QStringLiteral("measure/measure"), 400);
    const auto item = [&](const char *name) { return d.item(QString::fromLatin1(name), w->contentItem()); };

    // The setup: the layout's first speaker, short sweeps, no name (the core's default)
    d.waitFor(5000, [&] { return !mc->setupOutputs().isEmpty() && !mc->setupProblem().contains(QLatin1String("at least one")); });
    QList<int> on;
    QString speaker;
    for (const QVariant &v : mc->setupOutputs()) {
        const QVariantMap m = v.toMap();
        if (!m.value(QStringLiteral("on")).toBool()) continue;
        if (speaker.isEmpty()) speaker = m.value(QStringLiteral("speaker")).toString();
        if (m.value(QStringLiteral("speaker")).toString() == speaker && on.size() < 3) on << m.value(QStringLiteral("output")).toInt();
    }
    for (const QVariant &v : mc->setupOutputs()) {
        const int o = v.toMap().value(QStringLiteral("output")).toInt();
        mc->setOutputOn(o, on.contains(o));
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
    const bool ended = started && d.waitFor(110000, [&] {
        if (mc->measuring()) {
            barShown = barShown || (inView(w, item("Measure.stop")) && inView(w, item("Measure.runningText")));
            progressShown = progressShown || inView(w, item("Measure.progress"));
            if (barShown && progressShown && !shots()) return !mc->measuring();
        }
        return !mc->measuring() && !mc->running();
    });
    if (started && shots()) grabBoth(d, w, QStringLiteral("2-measuring"));
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
    grabBoth(d, w, QStringLiteral("4-proposal"));
    if (!proposed || tc->writes().isEmpty()) {
        d.notRun(QStringLiteral("ua2: the trial"), QStringLiteral("no proposal with a change (%1)").arg(tc->error()));
    } else {
        // The summed check keeps the page where the reader left it
        if (tc->needsSumCheck()) {
            QQuickItem *flick = tuneFlick(d, w);
            const qreal room = flick ? flick->property("contentHeight").toReal() - flick->height() : 0;
            if (flick && room > 40) {
                const qreal at = std::min<qreal>(room, 160);
                flick->setProperty("contentY", at);
                d.spin(100);
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
                const qreal now = flick->property("contentY").toReal();
                d.check(QStringLiteral("ua2: the summed check runs in the bar and keeps the page where it was (%1 → %2)").arg(at).arg(now),
                        ran && checkBar && std::abs(now - at) < 1.0);
            } else {
                d.notRun(QStringLiteral("ua2: the page kept across the summed check"), QStringLiteral("the page does not scroll"));
                tc->sumCheckRun();
                step(QStringLiteral("sumCheck"), 110000);
                d.waitFor(5000, [&] { return !mc->running(); });
            }
        }
        // Apply… asks, then the trial: the countdown, Keep primary, Revert plain, in view on every tab
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
    mc->clearError();
    writeVolume(d, volumeWas);
    w->resize(original);
    d.switchLayout("classic");
}

}   // namespace

static const SmokeDriver::Section measureBar("ua2-measurebar", 995, &measureBarSection);
