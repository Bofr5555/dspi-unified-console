// -- WP UB1 -- (UXP B1, owner D5) Crossovers and routing from the confirmed
// speaker layout, on the demo:
// - "ub1-layout": a stereo two-way layout (outputs 1-4: Left woofer, Left
//   tweeter, Right woofer, Right tweeter). Equaliser › Crossover's "From
//   Speaker Layout…" opens the sheet with one row for Left (woofer ◂▸ tweeter
//   at 2000 Hz, LR 24) and Right "Same as Left"; Apply sets outputs 1 and 3's
//   low passes and 2 and 4's high passes, and one undo puts all four back.
//   Routing's button routes L to outputs 1-2 and R to 3-4, one undo step. An
//   installer's 2.5 kHz high-pass floor on the tweeter: the sheet offers
//   2.5 kHz, refuses 2 kHz (its notice, Apply off) and apply() writes nothing;
//   a 6 dB/oct tweeter slope is refused. A three-way pair with a sub: two
//   transitions per side, the sub's low pass at 80 Hz, the sub fed L+R at
//   −6 dB. A layout not confirmed: the sheet says so and offers Speakers.
//   The partner default: a woofer's low pass chosen on the Crossover page
//   opens at its tweeter's high pass (2 kHz, not 1 kHz), in one undo step.
//   Settings › Speakers offers "Crossovers and Routing…" once confirmed.
//   Screenshots, dark and light, go to <out>/ub1/.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "HistoryController.h"
#include "LayoutSetupController.h"
#include "LimitsController.h"
#include "MatrixController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SidebarController.h"
#include "SpeakersController.h"

#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <cmath>
#include <functional>

namespace {

QQuickItem *shownItem(QQuickItem *from, const QString &name) {
    if (!from) return nullptr;
    if (from->objectName() == name && from->isVisible()) return from;
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children)
        if (QQuickItem *found = shownItem(child, name)) return found;
    return nullptr;
}

QStringList textsUnder(QQuickItem *from) {
    QStringList out;
    if (!from || !from->isVisible()) return out;
    if (from->inherits("QQuickText")) out << from->property("text").toString();
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children) out << textsUnder(child);
    return out;
}

QByteArray fill(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(1 << 16, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

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

bool near(double a, double b, double tol = 1.0) { return std::fabs(a - b) <= tol; }

QJsonObject row(int o, const char *role, const char *speaker, int group) {
    return QJsonObject{ { QStringLiteral("output"), o }, { QStringLiteral("role"), QLatin1String(role) },
                        { QStringLiteral("speaker"), QLatin1String(speaker) }, { QStringLiteral("group"), group },
                        { QStringLiteral("source"), QStringLiteral("user") } };
}

void layoutSection(SmokeDriver &d) {
    d.allowSectionTime(240);
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *cc = Registrar::instance<ChannelController>();
    auto *mc = Registrar::instance<MatrixController>();
    auto *lsc = Registrar::instance<LayoutSetupController>();
    auto *speakers = Registrar::instance<SpeakersController>();
    auto *history = Registrar::instance<HistoryController>();
    auto *limits = Registrar::instance<LimitsController>();
    auto *sidebar = Registrar::instance<SidebarController>();
    auto *settings = Registrar::instance<SettingsController>();
    QQuickWindow *w = d.mainWindow();
    d.check("ub1: the controllers and the window", cc && mc && lsc && speakers && history && limits && sidebar && settings && w);
    if (!cc || !mc || !lsc || !speakers || !history || !limits || !sidebar || !settings || !w) return;
    d.check("ub1: on the demo", d.waitFor(10000, [&] { return device->connected(); }) && d.waitIdle());
    const int inputs = cc->numInputs();
    const int outputs = cc->numOutputs();
    d.check(QStringLiteral("ub1: two inputs, seven outputs or more, crossovers (%1, %2)").arg(inputs).arg(outputs),
            inputs >= 2 && outputs >= 7 && cc->crossoverSupported());
    if (inputs < 2 || outputs < 7 || !cc->crossoverSupported()) return;
    const auto s = [&] { return device->state(); };
    const auto xo = [&](int o, int b) { return s()->xover[cc->outputChannel(o)][b]; };
    const auto settle = [&] {
        d.waitIdle();
        d.spin(80);
        d.waitIdle();
    };
    const auto undo = [&] {
        history->undo();
        settle();
    };
    // The first active pass of a kind on output o: {band, freq, type}, band -1 for none
    struct Pass {
        int band = -1;
        double freq = 0;
        int type = 0;
    };
    const auto pass = [&](int o, bool highPass) {
        for (int b = 0; b < ChannelController::XoBands; ++b) {
            const FilterParams f = xo(o, b);
            if (f.filter_type >= 32 && f.filter_type <= 63 && !f.bypass && cc->isLowPass(f.filter_type) != highPass)
                return Pass{ b, double(f.freq), f.filter_type };
        }
        return Pass{};
    };
    const auto flat = [&](int o) {
        for (int b = 0; b < ChannelController::XoBands; ++b)
            if (xo(o, b).filter_type != FILTER_FLAT) return false;
        return true;
    };
    const int lr24lp = ChannelController::crossoverType(1, 4, true), lr24hp = ChannelController::crossoverType(1, 4, false);

    // As it was: the crossovers, the routes and the layout, put back at the end
    QVector<QVector<FilterParams>> xoWas(outputs, QVector<FilterParams>(ChannelController::XoBands));
    struct Route {
        bool on;
        float gain;
        bool invert;
    };
    QVector<QVector<Route>> routesWas(outputs, QVector<Route>(inputs));
    for (int o = 0; o < outputs; ++o) {
        for (int b = 0; b < ChannelController::XoBands; ++b) xoWas[o][b] = xo(o, b);
        for (int i = 0; i < inputs; ++i) routesWas[o][i] = { s()->matrix_routing[i][o], s()->matrix_gain[i][o], s()->matrix_invert[i][o] };
    }
    const QJsonValue stored = onWorker(d, [](FfiCore *c) {
                                  return fill([c](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); });
                              }).value(QLatin1String("value"));
    const auto setLayout = [&](const QJsonObject &l) {
        const QByteArray text = QJsonDocument(l).toJson(QJsonDocument::Compact);
        onWorker(d, [text](FfiCore *c) {
            return fill([c, &text](char *b, uint32_t n) { return dspi_speaker_layout_set(c, text.constData(), b, n); });
        });
        speakers->reload();
        device->refresh();
        settle();
    };
    // Every output's crossovers off; L to output 1 alone, R to 2 alone (the first walk's start)
    const auto clean = [&] {
        for (int o = 0; o < outputs; ++o) cc->clearBands(cc->outputChannel(o), true);
        for (int o = 0; o < outputs; ++o)
            for (int i = 0; i < inputs; ++i) {
                const bool want = o == i && i < 2;
                if (s()->matrix_routing[i][o] != want) cc->setRoute(i, o, want, 0, false);
            }
        settle();
    };
    const QJsonObject twoWay{ { QStringLiteral("outputs"),
                                QJsonArray{ row(0, "low", "L", 0), row(1, "high", "L", 0), row(2, "low", "R", 1), row(3, "high", "R", 1) } },
                              { QStringLiteral("confirmedByUser"), true } };
    clean();
    setLayout(twoWay);
    d.check("ub1: the two-way layout is confirmed", d.waitFor(5000, [&] { return speakers->status() == QLatin1String("confirmed"); }));

    // ---- The plan ----
    QVariantMap plan = lsc->plan();
    QVariantList rows = plan.value(QStringLiteral("speakers")).toList();
    const auto transition = [&](int r, int t) {
        return rows.value(r).toMap().value(QStringLiteral("transitions")).toList().value(t).toMap();
    };
    d.check(QStringLiteral("ub1: nothing blocks (%1)").arg(plan.value(QStringLiteral("blocker")).toString()),
            plan.value(QStringLiteral("blocker")).toString().isEmpty());
    d.check(QStringLiteral("ub1: two speakers, Left then Right (%1)").arg(rows.size()),
            rows.size() == 2 && rows[0].toMap().value(QStringLiteral("title")).toString() == QLatin1String("Left")
                && rows[1].toMap().value(QStringLiteral("title")).toString() == QLatin1String("Right"));
    d.check(QStringLiteral("ub1: Left is woofer ◂▸ tweeter at 2000 Hz, LR 24, floor 1 kHz (%1, %2 Hz)")
                .arg(transition(0, 0).value(QStringLiteral("words")).toString())
                .arg(transition(0, 0).value(QStringLiteral("hz")).toDouble()),
            transition(0, 0).value(QStringLiteral("words")).toString() == QStringLiteral("woofer ◂▸ tweeter")
                && near(transition(0, 0).value(QStringLiteral("hz")).toDouble(), 2000, 0.1)
                && transition(0, 0).value(QStringLiteral("family")).toInt() == 1 && transition(0, 0).value(QStringLiteral("order")).toInt() == 4
                && near(transition(0, 0).value(QStringLiteral("floorHz")).toDouble(), 1000, 0.1));
    d.check("ub1: Right is the same shape as Left", rows.value(1).toMap().value(QStringLiteral("sameAs")).toInt() == 0);
    d.check(QStringLiteral("ub1: the floor line (%1)").arg(plan.value(QStringLiteral("floorText")).toString()),
            plan.value(QStringLiteral("floorText")).toString() == QStringLiteral("Tweeter high-passes never below 1 kHz."));
    const QVariantList routeLines = plan.value(QStringLiteral("routes")).toList();
    d.check(QStringLiteral("ub1: L feeds Left woofer and tweeter, R the Right pair (%1)").arg(routeLines.size()),
            routeLines.size() == 2 && routeLines[0].toMap().value(QStringLiteral("input")).toInt() == 0
                && routeLines[0].toMap().value(QStringLiteral("outputs")).toStringList()
                       == QStringList{ QStringLiteral("Left woofer · 1"), QStringLiteral("Left tweeter · 2") }
                && routeLines[1].toMap().value(QStringLiteral("input")).toInt() == 1);

    QDir(d.outDir()).mkpath(QStringLiteral("ub1"));
    const QSize original = w->size();
    const Qt::ColorScheme scheme = shell->colorSchemeOverride();
    if (!d.switchLayout("modern")) {
        d.check("ub1: in Modern", false);
        return;
    }
    if (auto *notices = Registrar::instance<NoticeCenter>()) notices->clear();
    w->resize(QSize(1600, 1000));
    d.waitFor(1000, [&] { return w->contentItem()->width() == 1600; });
    sidebar->select(cc->outputChannel(0));
    settle();
    const auto shown = [&](const QString &name) { return shownItem(w->contentItem(), name); };
    const auto press = [&](const QString &name) {
        QQuickItem *it = shown(name);
        return it && SmokeDriver::press(it) && (d.spin(250), true);
    };
    const auto shoot = [&](const QString &name, const QString &schemeName) {
        d.spin(250);
        d.check(QStringLiteral("ub1: screenshot %1-%2").arg(name, schemeName), d.grab(w, QStringLiteral("ub1/%1-%2.png").arg(name, schemeName)));
    };
    const auto sheetOpen = [&] { return d.waitFor(2000, [&] { return shown(QStringLiteral("CrossoverFromLayout.apply")) != nullptr; }); };
    const auto closeSheet = [&] {
        press(QStringLiteral("CrossoverFromLayout.cancel"));
        d.waitFor(2000, [&] { return !shown(QStringLiteral("CrossoverFromLayout.apply")); });
    };
    const auto typeInto = [&](const QString &name, const char *value) {
        QQuickItem *f = shown(name + QStringLiteral(".field"));
        if (!f) return false;
        f->forceActiveFocus(Qt::TabFocusReason);
        d.spin(20);
        for (const char *c = value; *c; ++c) QTest::keyClick(w, *c);
        QTest::keyClick(w, Qt::Key_Return);
        d.spin(150);
        return true;
    };

    for (const char *schemeName : { "dark", "light" }) {
        const QString sn = QLatin1String(schemeName);
        const bool first = sn == QLatin1String("dark");
        shell->setColorSchemeOverride(first ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
        d.spin(300);
        clean();

        // ---- Equaliser › Crossover: the tool row, the sheet, Apply, one undo ----
        d.goTo(QStringLiteral("equaliser/crossover"), 300);
        d.check(QStringLiteral("ub1: the Crossover page has From Speaker Layout… (%1)").arg(sn), shown(QStringLiteral("EqualiserCrossover.fromLayout")) != nullptr);
        shoot(QStringLiteral("crossover-page"), sn);
        press(QStringLiteral("EqualiserCrossover.fromLayout"));
        d.check(QStringLiteral("ub1: the sheet opens (%1)").arg(sn), sheetOpen());
        if (first) {
            QQuickItem *same = shown(QStringLiteral("CrossoverFromLayout.speaker.1.same"));
            d.check("ub1: Right says Same as Left, ticked, and has no fields of its own",
                    same && same->property("checked").toBool() && same->property("text").toString() == QLatin1String("Same as Left")
                        && !shown(QStringLiteral("CrossoverFromLayout.speaker.1.transition.0.freq")));
            const QStringList words = textsUnder(shown(QStringLiteral("CrossoverFromLayout.speaker.0.transition.0")));
            d.check(QStringLiteral("ub1: Left's row reads Left, woofer ◂▸ tweeter (%1)").arg(words.join(QLatin1Char('|'))),
                    words.contains(QStringLiteral("Left")) && words.contains(QStringLiteral("woofer ◂▸ tweeter")));
            QQuickItem *slope = shown(QStringLiteral("CrossoverFromLayout.speaker.0.transition.0.slope"));
            d.check(QStringLiteral("ub1: its slope shows LR 24 (%1)").arg(slope ? slope->property("displayText").toString() : QString()),
                    slope && slope->property("displayText").toString() == QLatin1String("LR 24"));
            QQuickItem *crossovers = shown(QStringLiteral("CrossoverFromLayout.crossovers"));
            QQuickItem *routing = shown(QStringLiteral("CrossoverFromLayout.routing"));
            d.check("ub1: from the Crossover page the crossovers are ticked, the routing not",
                    crossovers && crossovers->property("checked").toBool() && routing && !routing->property("checked").toBool());
        }
        shoot(QStringLiteral("sheet-crossover"), sn);
        const QVariant before = history->entries().value(0);
        press(QStringLiteral("CrossoverFromLayout.apply"));
        settle();
        d.check(QStringLiteral("ub1: Apply closes the sheet (%1)").arg(sn), !shown(QStringLiteral("CrossoverFromLayout.apply")));
        const bool set = d.waitFor(4000, [&] {
            return pass(0, false).type == lr24lp && near(pass(0, false).freq, 2000) && pass(1, true).type == lr24hp && near(pass(1, true).freq, 2000)
                && pass(2, false).type == lr24lp && near(pass(2, false).freq, 2000) && pass(3, true).type == lr24hp && near(pass(3, true).freq, 2000);
        });
        d.check(QStringLiteral("ub1: Out 1 and 3 low-pass, Out 2 and 4 high-pass at 2 kHz, LR 24 (%1)").arg(sn), set);
        d.check("ub1: the woofers get no high pass, the tweeters no low pass",
                pass(0, true).band < 0 && pass(1, false).band < 0 && pass(2, true).band < 0 && pass(3, false).band < 0);
        d.check("ub1: the routing is as it was (not ticked)", s()->matrix_routing[0][0] && !s()->matrix_routing[0][1] && s()->matrix_routing[1][1]);
        d.waitFor(2000, [&] { return history->entries().value(1) == before; });
        d.check(QStringLiteral("ub1: one history step, \"From Speaker Layout\" (%1)")
                    .arg(history->entries().value(0).toMap().value(QStringLiteral("title")).toString()),
                history->entries().value(1) == before
                    && history->entries().value(0).toMap().value(QStringLiteral("title")).toString().contains(QLatin1String("From Speaker Layout")));
        shoot(QStringLiteral("crossover-after"), sn);
        undo();
        d.check(QStringLiteral("ub1: one undo puts all four back (%1)").arg(sn),
                d.waitFor(4000, [&] { return flat(0) && flat(1) && flat(2) && flat(3); }));

        // ---- Routing: its button, the routing part, one undo ----
        d.goTo(QStringLiteral("routing/matrix"), 300);
        press(QStringLiteral("RoutingTools.fromLayout"));
        d.check(QStringLiteral("ub1: Routing's From Speaker Layout… opens the sheet (%1)").arg(sn), sheetOpen());
        if (first) {
            QQuickItem *crossovers = shown(QStringLiteral("CrossoverFromLayout.crossovers"));
            QQuickItem *routing = shown(QStringLiteral("CrossoverFromLayout.routing"));
            d.check("ub1: from Routing the routing is ticked, the crossovers not",
                    crossovers && !crossovers->property("checked").toBool() && routing && routing->property("checked").toBool());
            const QStringList words = textsUnder(shown(QStringLiteral("CrossoverFromLayout.route.0")));
            d.check(QStringLiteral("ub1: the first input feeds Left woofer · 1, Left tweeter · 2 (%1)").arg(words.join(QLatin1Char('|'))),
                    words.contains(QStringLiteral("→ Left woofer · 1, Left tweeter · 2")));
        }
        shoot(QStringLiteral("sheet-routing"), sn);
        press(QStringLiteral("CrossoverFromLayout.apply"));
        const bool routed = d.waitFor(4000, [&] {
            const DspState *st = s();
            return st->matrix_routing[0][0] && st->matrix_routing[0][1] && !st->matrix_routing[0][2] && !st->matrix_routing[0][3]
                && !st->matrix_routing[1][0] && !st->matrix_routing[1][1] && st->matrix_routing[1][2] && st->matrix_routing[1][3];
        });
        d.check(QStringLiteral("ub1: L feeds Out 1-2, R Out 3-4, and R no longer Out 2 (%1)").arg(sn), routed);
        d.check("ub1: the crossovers stay off (not ticked)", flat(0) && flat(1));
        shoot(QStringLiteral("routing-after"), sn);
        undo();
        d.check(QStringLiteral("ub1: one undo puts the routes back (%1)").arg(sn), d.waitFor(4000, [&] {
                    const DspState *st = s();
                    return st->matrix_routing[0][0] && !st->matrix_routing[0][1] && st->matrix_routing[1][1] && !st->matrix_routing[1][2];
                }));

        // ---- The installer's floor: 2.5 kHz on Left tweeter · 2 ----
        limits->setOutputLimit(1, QStringLiteral("highpassMinHz"), 2500);
        d.waitFor(8000, [&] { return !limits->busy(); });
        settle();
        if (first) {
            plan = lsc->plan();
            rows = plan.value(QStringLiteral("speakers")).toList();
            d.check(QStringLiteral("ub1: with the limit the tweeter's floor is 2.5 kHz and the sheet offers 2.5 kHz (%1, %2)")
                        .arg(transition(0, 0).value(QStringLiteral("floorHz")).toDouble())
                        .arg(transition(0, 0).value(QStringLiteral("hz")).toDouble()),
                    near(transition(0, 0).value(QStringLiteral("floorHz")).toDouble(), 2500, 0.1)
                        && transition(0, 0).value(QStringLiteral("hz")).toDouble() >= 2500);
            d.check(QStringLiteral("ub1: the floor line names the limits (%1)").arg(plan.value(QStringLiteral("floorText")).toString()),
                    plan.value(QStringLiteral("floorText")).toString() == QStringLiteral("Tweeter high-passes never below 2.5 kHz (the installer's limits)."));
            const auto values = [](double left, double right, int order) {
                const auto t = [order](double hz) {
                    return QVariantMap{ { QStringLiteral("transitions"), QVariantList{ QVariantMap{ { QStringLiteral("hz"), hz }, { QStringLiteral("family"), order == 1 ? 2 : 1 }, { QStringLiteral("order"), order } } } } };
                };
                return QVariantList{ t(left), t(right) };
            };
            const QString low = lsc->check(values(2000, 2500, 4));
            d.check(QStringLiteral("ub1: 2 kHz on Left is refused (%1)").arg(low), low == QStringLiteral("Left: a tweeter's high pass never goes below 2.5 kHz."));
            const QString gentle = lsc->check(values(3000, 3000, 1));
            d.check(QStringLiteral("ub1: a 6 dB/oct tweeter slope is refused (%1)").arg(gentle),
                    gentle == QStringLiteral("Left: a tweeter's high pass needs 12 dB/oct or steeper."));
            const Pass held = pass(1, true);
            d.check(QStringLiteral("ub1: apply() refuses 2 kHz and writes nothing"),
                    lsc->apply(values(2000, 2500, 4), true, false) == low && (settle(), pass(0, false).band < 0 && pass(1, true).band == held.band
                                                                                         && near(pass(1, true).freq, held.freq)));
            d.check("ub1: 2.5 kHz on both passes the check", lsc->check(values(2500, 2500, 4)).isEmpty());
        }
        d.goTo(QStringLiteral("equaliser/crossover"), 300);
        press(QStringLiteral("EqualiserCrossover.fromLayout"));
        sheetOpen();
        typeInto(QStringLiteral("CrossoverFromLayout.speaker.0.transition.0.freq"), "2000");
        QQuickItem *apply = shown(QStringLiteral("CrossoverFromLayout.apply"));
        const QStringList problem = textsUnder(shown(QStringLiteral("CrossoverFromLayout.problem")));
        d.check(QStringLiteral("ub1: typed 2000, the sheet says why and Apply waits (%1: %2)").arg(sn, problem.join(QLatin1Char('|'))),
                problem.contains(QStringLiteral("Left: a tweeter's high pass never goes below 2.5 kHz.")) && apply && !apply->isEnabled());
        shoot(QStringLiteral("sheet-floor"), sn);
        closeSheet();
        limits->clearLimits();
        d.waitFor(8000, [&] { return !limits->busy(); });
        clean();

        // ---- The partner default on the Crossover page ----
        cc->setBandType(cc->outputChannel(1), 0, true, lr24hp, 2000);
        settle();
        d.goTo(QStringLiteral("equaliser/crossover"), 300);
        const QVariant stepBefore = history->entries().value(0);
        QQuickItem *lp = shown(QStringLiteral("EqualiserCrossover.row.0.lowPass.type"));
        int index = -1;
        if (lp) {
            const QStringList choices = lp->property("model").toStringList();
            for (int i = 0; i < choices.size(); ++i)
                if (choices[i].startsWith(QLatin1String("Linkwitz-Riley")) && choices[i].contains(QLatin1String("24 "))) index = i;
            QMetaObject::invokeMethod(lp, "activated", Q_ARG(int, index));
        }
        settle();
        d.check(QStringLiteral("ub1: the woofer's new low pass opens at its tweeter's 2 kHz (%1: %2 Hz)").arg(sn).arg(pass(0, false).freq),
                index > 0 && d.waitFor(3000, [&] { return pass(0, false).type == lr24lp && near(pass(0, false).freq, 2000); }));
        d.check("ub1: as one step", d.waitFor(2000, [&] { return history->entries().value(1) == stepBefore; }));
        if (first) {
            QQuickItem *legend = shown(QStringLiteral("EqualiserCrossover.legend"));
            QQuickItem *brief = shown(QStringLiteral("EqualiserCrossover.row.0.lowPass.type"));
            d.check(QStringLiteral("ub1: the table says LR 24, the legend explains it (%1; %2)")
                        .arg(brief ? brief->property("displayText").toString() : QString(), legend ? legend->property("text").toString() : QString()),
                    brief && brief->property("displayText").toString() == QLatin1String("LR 24") && legend
                        && legend->property("text").toString().startsWith(QLatin1String("LR 24 is Linkwitz-Riley at 24 dB/oct")));
        }
        undo();
        d.check("ub1: one undo takes the low pass away, the tweeter's high pass stays",
                d.waitFor(3000, [&] { return pass(0, false).band < 0 && pass(1, true).band >= 0; }));
        clean();
    }

    // ---- A three-way pair and a sub ----
    const QJsonObject threeWay{ { QStringLiteral("outputs"),
                                  QJsonArray{ row(0, "low", "L", 0), row(1, "mid", "L", 0), row(2, "high", "L", 0), row(3, "low", "R", 1),
                                              row(4, "mid", "R", 1), row(5, "high", "R", 1), row(6, "sub", "Sub", 2) } },
                                { QStringLiteral("confirmedByUser"), true } };
    setLayout(threeWay);
    d.waitFor(5000, [&] { return speakers->status() == QLatin1String("confirmed"); });
    plan = lsc->plan();
    rows = plan.value(QStringLiteral("speakers")).toList();
    d.check(QStringLiteral("ub1: three-way: Left, Right, Sub (%1)").arg(rows.size()),
            rows.size() == 3 && rows[2].toMap().value(QStringLiteral("title")).toString() == QLatin1String("Sub"));
    d.check(QStringLiteral("ub1: Left has woofer ◂▸ mid driver at 400 Hz, mid driver ◂▸ tweeter at 3 kHz (%1, %2)")
                .arg(transition(0, 0).value(QStringLiteral("words")).toString(), transition(0, 1).value(QStringLiteral("words")).toString()),
            rows.value(0).toMap().value(QStringLiteral("transitions")).toList().size() == 2
                && transition(0, 0).value(QStringLiteral("words")).toString() == QStringLiteral("woofer ◂▸ mid driver")
                && near(transition(0, 0).value(QStringLiteral("hz")).toDouble(), 400, 0.1)
                && near(transition(0, 0).value(QStringLiteral("floorHz")).toDouble(), 80, 0.1)
                && transition(0, 1).value(QStringLiteral("words")).toString() == QStringLiteral("mid driver ◂▸ tweeter")
                && near(transition(0, 1).value(QStringLiteral("hz")).toDouble(), 3000, 0.1));
    d.check(QStringLiteral("ub1: the sub gets its low pass at 80 Hz (%1)").arg(transition(2, 0).value(QStringLiteral("words")).toString()),
            transition(2, 0).value(QStringLiteral("words")).toString() == QStringLiteral("low pass")
                && near(transition(2, 0).value(QStringLiteral("hz")).toDouble(), 80, 0.1));
    QVariantList values;
    for (const QVariant &r : rows) {
        QVariantList ts;
        for (const QVariant &t : r.toMap().value(QStringLiteral("transitions")).toList()) ts << t;
        values << QVariantMap{ { QStringLiteral("transitions"), ts } };
    }
    d.check("ub1: apply() takes the three-way and its routing", lsc->apply(values, true, true).isEmpty());
    settle();
    d.check(QStringLiteral("ub1: the mid driver has a high pass at 400 Hz and a low pass at 3 kHz in two bands (%1, %2)")
                .arg(pass(1, true).freq).arg(pass(1, false).freq),
            d.waitFor(4000, [&] {
                return near(pass(1, true).freq, 400) && near(pass(1, false).freq, 3000) && pass(1, true).band != pass(1, false).band
                    && near(pass(0, false).freq, 400) && near(pass(2, true).freq, 3000) && near(pass(6, false).freq, 80)
                    && pass(6, true).band < 0;
            }));
    d.check(QStringLiteral("ub1: the sub is fed L+R at −6 dB each (%1, %2)").arg(s()->matrix_gain[0][6]).arg(s()->matrix_gain[1][6]),
            s()->matrix_routing[0][6] && s()->matrix_routing[1][6] && near(s()->matrix_gain[0][6], -6, 0.01) && near(s()->matrix_gain[1][6], -6, 0.01));
    undo();
    d.check("ub1: one undo puts the seven outputs back",
            d.waitFor(4000, [&] { return flat(0) && flat(1) && flat(2) && flat(5) && flat(6) && !s()->matrix_routing[0][6]; }));

    // ---- A layout not confirmed ----
    QJsonObject unconfirmed = twoWay;
    unconfirmed.insert(QStringLiteral("confirmedByUser"), false);
    setLayout(unconfirmed);
    d.waitFor(5000, [&] { return speakers->status() != QLatin1String("confirmed"); });
    plan = lsc->plan();
    d.check(QStringLiteral("ub1: an unconfirmed layout blocks the sheet (%1)").arg(plan.value(QStringLiteral("blocker")).toString()),
            plan.value(QStringLiteral("blockerKind")).toString() == QLatin1String("layout") && plan.value(QStringLiteral("speakers")).toList().isEmpty());
    d.check("ub1: and apply() refuses", !lsc->apply({}, false, true).isEmpty());
    for (const char *schemeName : { "dark", "light" }) {
        shell->setColorSchemeOverride(QLatin1String(schemeName) == QLatin1String("dark") ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
        d.goTo(QStringLiteral("equaliser/crossover"), 300);
        press(QStringLiteral("EqualiserCrossover.fromLayout"));
        sheetOpen();
        QQuickItem *apply = shown(QStringLiteral("CrossoverFromLayout.apply"));
        QQuickItem *blocker = shown(QStringLiteral("CrossoverFromLayout.blocker"));
        d.check(QStringLiteral("ub1: the sheet says to confirm first, offers Speakers, Apply off (%1)").arg(QLatin1String(schemeName)),
                blocker && textsUnder(blocker).join(QLatin1Char('|')).contains(QLatin1String("Confirm the speaker layout first"))
                    && shown(QStringLiteral("CrossoverFromLayout.blocker.action")) && apply && !apply->isEnabled());
        shoot(QStringLiteral("sheet-unconfirmed"), QLatin1String(schemeName));
        closeSheet();
    }

    // ---- Settings › Speakers offers it once the layout is confirmed ----
    setLayout(twoWay);
    d.waitFor(5000, [&] { return speakers->status() == QLatin1String("confirmed"); });
    for (const char *schemeName : { "dark", "light" }) {
        shell->setColorSchemeOverride(QLatin1String(schemeName) == QLatin1String("dark") ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
        d.goTo(QStringLiteral("settings/device"), 200);
        settings->open(QStringLiteral("speakers"));
        settle();
        d.spin(300);
        d.check(QStringLiteral("ub1: Settings › Speakers offers Crossovers and Routing… (%1)").arg(QLatin1String(schemeName)),
                d.waitFor(3000, [&] { return shown(QStringLiteral("SpeakersPage.fromLayout")) != nullptr; }));
        shoot(QStringLiteral("speakers"), QLatin1String(schemeName));
        press(QStringLiteral("SpeakersPage.fromLayout"));
        if (sheetOpen()) {
            QQuickItem *crossovers = shown(QStringLiteral("CrossoverFromLayout.crossovers"));
            QQuickItem *routing = shown(QStringLiteral("CrossoverFromLayout.routing"));
            d.check(QStringLiteral("ub1: from Speakers both parts are ticked (%1)").arg(QLatin1String(schemeName)),
                    crossovers && crossovers->property("checked").toBool() && routing && routing->property("checked").toBool());
            shoot(QStringLiteral("sheet-speakers"), QLatin1String(schemeName));
            closeSheet();
        } else {
            d.check("ub1: the sheet from Speakers", false);
        }
    }

    // As it was: the layout, the crossovers, the routes
    shell->setColorSchemeOverride(scheme);
    w->resize(original);
    d.goTo(QStringLiteral("overview/overview"));
    if (stored.isObject()) setLayout(stored.toObject());
    for (int o = 0; o < outputs; ++o) {
        for (int b = 0; b < ChannelController::XoBands; ++b) {
            const FilterParams f = xoWas[o][b];
            const uint8_t ch = uint8_t(cc->outputChannel(o)), band = uint8_t(b);
            device->write(QStringLiteral("ub1 restore %1 %2").arg(o).arg(b), [ch, band, f](FfiCore *c) { return dspi_set_crossover(c, ch, band, f); });
        }
        for (int i = 0; i < inputs; ++i)
            if (s()->matrix_routing[i][o] != routesWas[o][i].on || routesWas[o][i].on)
                cc->setRoute(i, o, routesWas[o][i].on, routesWas[o][i].gain, routesWas[o][i].invert);
    }
    device->refresh();
    settle();
    d.switchLayout("classic");
}

const SmokeDriver::Section layout("ub1-layout", 992, &layoutSection);

} // namespace
