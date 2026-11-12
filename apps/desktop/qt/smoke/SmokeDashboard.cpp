// -- WP QP5 -- The Overview: its cards against the core's card plan, the band
// rows' texts, the delay readout, cards per row and the layout popup, the
// meters following the meter poll, the stale note, and no device.
#include "SmokeDriver.h"

#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "DashboardController.h"
#include "DeviceModel.h"
#include "Poller.h"
#include "Snapshot.h"
#include "render/Meter.h"

#include <QAccessible>
#include <QJsonArray>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTest>
#include <cmath>
#include <functional>

namespace {

QString shown(QObject *o) { return o ? o->property("shownName").toString() : QString(); }

QString textOf(QObject *o) { return o ? o->property("text").toString() : QString(); }

QAccessible::Role roleOf(QQuickItem *item) {
    QAccessibleInterface *a = item ? QAccessible::queryAccessibleInterface(item) : nullptr;
    return a ? a->role() : QAccessible::NoRole;
}

// The core's output cards, read on the worker as the controller does
QJsonArray coreCards(SmokeDriver &d) {
    QJsonArray cards;
    bool done = false;
    d.app().core->worker()->call(
        d.app().device,
        [](FfiCore *c) {
            return jsonValue(readSized([c](char *b, uint32_t n) { return dspi_output_cards(c, b, n); })).toArray();
        },
        [&](const QJsonArray &a) {
            cards = a;
            done = true;
        });
    d.waitFor(3000, [&]() { return done; });
    return cards;
}

void checkTexts(SmokeDriver &d) {
    using C = DashboardController;
    d.check(QStringLiteral("qp5: type codes are the Mac's"),
            C::typeCode(FILTER_FLAT) == QLatin1String("OFF") && C::typeCode(FILTER_PEAKING) == QLatin1String("PK") &&
                C::typeCode(FILTER_LOWPASS) == QLatin1String("HC") && C::typeCode(FILTER_HIGHPASS) == QLatin1String("LC") &&
                C::typeCode(FILTER_NOTCH) == QLatin1String("NO") && C::typeCode(FILTER_LOWSHELF1) == QLatin1String("LS1"));
    d.check(QStringLiteral("qp5: gain shows on bells and shelves only"),
            C::usesGain(FILTER_PEAKING) && C::usesGain(FILTER_HIGHSHELF1) && !C::usesGain(FILTER_LOWPASS) &&
                !C::usesGain(FILTER_NOTCH));
    d.check(QStringLiteral("qp5: Q shows on a bell only"), C::showsQ(FILTER_PEAKING) && !C::showsQ(FILTER_LOWSHELF));
    d.check(QStringLiteral("qp5: gain text is signed with zeros trimmed"),
            C::gainText(3) == QLatin1String("+3.0") && C::gainText(-5.5) == QLatin1String("-5.5") &&
                C::gainText(1.25) == QLatin1String("+1.25") && C::gainText(0) == QLatin1String("+0.0"));
    d.check(QStringLiteral("qp5: Q text has up to three decimals"),
            C::qText(0.707) == QLatin1String("0.707") && C::qText(1) == QLatin1String("1.0"));
    d.check(QStringLiteral("qp5: frequency text is whole hertz"), C::freqText(1000.4) == QLatin1String("1000"));
    d.check(QStringLiteral("qp5: delay text drops trailing zeros"),
            C::delayText(1.5) == QLatin1String("1.5") && C::delayText(0) == QLatin1String("0") &&
                C::delayText(2.0625) == QLatin1String("2.0625"));
}

void checkCards(SmokeDriver &d) {
    DashboardController *dc = Registrar::instance<DashboardController>();
    DeviceModel *device = d.app().device;
    QQuickItem *dash = d.item(QStringLiteral("Dashboard"));
    d.check(QStringLiteral("qp5: the Overview shows with a device"),
            dash && dash->isVisible() && d.waitFor(2000, [&]() { return dash->opacity() > 0.99; }));
    d.check(QStringLiteral("qp5: the Overview is a pane"), roleOf(dash) == QAccessible::Pane);
    d.waitIdle();

    const QJsonArray core = coreCards(d);
    d.check(QStringLiteral("qp5: the core gives output cards"), !core.isEmpty());
    d.check(QStringLiteral("qp5: one input card and the core's output cards"),
            d.waitFor(2000, [&]() { return dc->cards().size() == core.size() + 1; }));

    QQuickItem *input = d.item(QStringLiteral("Dashboard.card.input"));
    d.check(QStringLiteral("qp5: the input card is first and shown"), input && input->isVisible());
    d.check(QStringLiteral("qp5: a card is a group"), roleOf(input) == QAccessible::Grouping);
    const ChannelListModel *rows = device->channels();
    for (int ch = 0; ch < 2; ++ch) {
        QObject *col = d.object(QStringLiteral("Dashboard.column.%1").arg(ch));
        d.check(QStringLiteral("qp5: input %1's column heading is its sidebar label").arg(ch + 1),
                col && shown(col) == rows->get(rows->rowOfChannel(ch), QStringLiteral("label")).toString());
        d.check(QStringLiteral("qp5: an input column has no delay readout"),
                textOf(d.object(QStringLiteral("Dashboard.delay.%1").arg(ch))).isEmpty());
    }

    const int inputs = device->state() ? int(device->state()->num_input_channels) : 0;
    bool headings = true, shownCards = true;
    for (const QJsonValue &v : core) {
        const QJsonObject o = v.toObject();
        const int left = o.value(QLatin1String("left")).toInt();
        const bool pair = o.value(QLatin1String("right")).isDouble();
        const int right = pair ? o.value(QLatin1String("right")).toInt() : -1;
        const QString key = pair ? QStringLiteral("out%1-%2").arg(left).arg(right) : QStringLiteral("out%1").arg(left);
        QQuickItem *card = d.item(QStringLiteral("Dashboard.card.") + key);
        shownCards = shownCards && card && card->isVisible();
        headings = headings && shown(d.object(QStringLiteral("Dashboard.column.%1").arg(inputs + left))) ==
                                   o.value(QLatin1String("leftName")).toString();
        if (pair)
            headings = headings && shown(d.object(QStringLiteral("Dashboard.column.%1").arg(inputs + right))) ==
                                       o.value(QLatin1String("rightName")).toString();
    }
    d.check(QStringLiteral("qp5: every output card of the core is shown"), shownCards);
    d.check(QStringLiteral("qp5: output columns carry the core's headings"), headings);

    // Ten band rows under each column
    QQuickItem *firstOut = d.item(QStringLiteral("Dashboard.column.%1").arg(inputs));
    std::function<int(QQuickItem *)> countRows = [&](QQuickItem *at) {
        int n = at->property("band").isValid() && at->property("tone").isValid() ? 1 : 0;
        for (QQuickItem *child : at->childItems()) n += countRows(child);
        return n;
    };
    const int bandRows = firstOut ? countRows(firstOut) : 0;
    d.check(QStringLiteral("qp5: a column has ten band rows"), bandRows == BANDS_PER_CHANNEL);
}

void checkDelay(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    const DspState *s = device->state();
    if (!s) {
        d.check(QStringLiteral("qp5: a state to read the delay from"), false);
        return;
    }
    const int inputs = int(s->num_input_channels);
    const float before = s->output_delay_ms[0];
    bool ok = false;
    device->write(QStringLiteral("output delay 0"), [](FfiCore *c) { return dspi_set_output_delay(c, 0, 1.5f); },
                  [&](bool r) { ok = r; });
    d.waitIdle();
    QObject *delay = d.object(QStringLiteral("Dashboard.delay.%1").arg(inputs));
    d.check(QStringLiteral("qp5: an output delay write is taken"), ok);
    d.check(QStringLiteral("qp5: the output column shows its delay"),
            d.waitFor(2000, [&]() { return textOf(delay) == QLatin1String("Delay: 1.5ms"); }));
    device->write(QStringLiteral("output delay 0"), [before](FfiCore *c) { return dspi_set_output_delay(c, 0, before); });
    d.waitIdle();
}

void checkLayout(SmokeDriver &d) {
    DashboardController *dc = Registrar::instance<DashboardController>();
    QQuickItem *grid = d.item(QStringLiteral("Dashboard.grid"));
    const int saved = dc->cardsPerRow();
    const int count = int(dc->cards().size());

    dc->setCardsPerRow(1);
    d.check(QStringLiteral("qp5: one card per row"), grid && grid->property("columns").toInt() == 1);
    d.check(QStringLiteral("qp5: the choice is saved"), QSettings().value(QStringLiteral("dashboard/cardsPerRow")).toInt() == 1);
    dc->setCardsPerRow(3);
    d.check(QStringLiteral("qp5: three per row, never more than the cards"),
            grid && grid->property("columns").toInt() == qMin(3, count));
    dc->setCardsPerRow(7);
    d.check(QStringLiteral("qp5: a choice past three is held at three"), dc->cardsPerRow() == 3);
    dc->setCardsPerRow(0);
    d.check(QStringLiteral("qp5: Auto fits 440 px cards with 18 px gaps"),
            dc->columnsFor(0, 440, 5) == 1 && dc->columnsFor(0, 897, 5) == 1 && dc->columnsFor(0, 898, 5) == 2 &&
                dc->columnsFor(0, 1800, 2) == 2 && dc->columnsFor(0, 10, 5) == 1 && dc->columnsFor(2, 10, 5) == 2 &&
                dc->columnsFor(3, 2000, 2) == 2);
    d.check(QStringLiteral("qp5: Auto follows the grid's width"),
            grid && grid->property("columns").toInt() == dc->columnsFor(0, grid->width(), count));

    // The gear and the popup
    QQuickWindow *w = d.mainWindow();
    QQuickItem *card = d.item(QStringLiteral("Dashboard.card.input"));
    QQuickItem *gear = card ? d.item(QStringLiteral("Dashboard.gear"), card) : nullptr;
    d.check(QStringLiteral("qp5: the gear is hidden until the pointer is over the card"), gear && gear->opacity() < 0.01);
    if (!w || !card || !gear) return;
    // A user clicks the gear in the active main window (an earlier section may
    // have left another window active)
    w->requestActivate();
    d.check(QStringLiteral("qp5: the main window is active"), QTest::qWaitForWindowActive(w, 2000));
    QTest::mouseMove(w, card->mapToScene(QPointF(card->width() / 2, card->height() / 2)).toPoint());
    d.check(QStringLiteral("qp5: the gear shows while the pointer is over the card"),
            d.waitFor(1000, [&]() { return gear->opacity() > 0.99; }));
    d.check(QStringLiteral("qp5: the gear is named for screen readers"),
            gear->property("toolTip").toString() == QLatin1String("Dashboard layout"));
    QObject *popup = gear->findChild<QObject *>(QStringLiteral("DashboardLayoutPopup"));
    QPoint gearAt = gear->mapToScene(QPointF(gear->width() / 2, gear->height() / 2)).toPoint();
    QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, gearAt);
    d.check(QStringLiteral("qp5: the gear opens the layout popup"),
            popup && d.waitFor(1000, [&]() { return popup->property("opened").toBool(); }));
    if (!popup) return;
    d.spin(200);
    d.check(QStringLiteral("qp5: screenshot of the layout popup"), d.grab(w, QStringLiteral("window-dashboard-layout.png")));

    auto *content = qobject_cast<QQuickItem *>(popup->property("contentItem").value<QObject *>());
    QQuickItem *summary = content ? d.item(QStringLiteral("DashboardLayoutPopup.summary"), content) : nullptr;
    d.check(QStringLiteral("qp5: Auto says it fits the window"),
            textOf(summary) == QLatin1String("Fits as many cards per row as the window allows."));
    QQuickItem *two = content ? d.item(QStringLiteral("DashboardLayoutPopup.tile.2"), content) : nullptr;
    d.check(QStringLiteral("qp5: a tile is a radio button"), roleOf(two) == QAccessible::RadioButton);
    if (two) QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, two->mapToScene(QPointF(two->width() / 2, two->height() / 2)).toPoint());
    d.check(QStringLiteral("qp5: the 2 tile chooses two per row"), dc->cardsPerRow() == 2);
    d.check(QStringLiteral("qp5: the summary follows"), textOf(summary) == QLatin1String("Up to 2 cards per row."));
    d.check(QStringLiteral("qp5: the popup stays open on a choice"), popup->property("opened").toBool());

    // One click on the gear while the popup is open closes it, once (behaviour.md §26)
    // (each click well apart from the last, so none of them is a double
    // click; the choice moved the card, so the gear is found again)
    d.spin(600);
    gearAt = gear->mapToScene(QPointF(gear->width() / 2, gear->height() / 2)).toPoint();
    QTest::mouseMove(w, gearAt);
    QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, gearAt);
    d.spin(600);
    d.check(QStringLiteral("qp5: a click on the gear closes the open popup and leaves it closed"),
            !popup->property("opened").toBool());
    // Hover is only worked out again when the pointer moves, as it would
    QTest::mouseMove(w, gearAt + QPoint(1, 0));
    QTest::mouseMove(w, gearAt);
    d.check(QStringLiteral("qp5: the gear still shows under the pointer"), gear->opacity() > 0.5 && gear->isEnabled());
    QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, gearAt);
    d.check(QStringLiteral("qp5: the gear opens it again"), d.waitFor(1000, [&]() { return popup->property("opened").toBool(); }));
    QTest::keyClick(w, Qt::Key_Escape);
    d.check(QStringLiteral("qp5: Escape closes the popup"), d.waitFor(1000, [&]() { return !popup->property("opened").toBool(); }));

    dc->setCardsPerRow(saved);
    QTest::mouseMove(w, QPoint(2, 2));
}

void checkMeters(SmokeDriver &d) {
    Poller *poller = Poller::instance();
    DashboardController *dc = Registrar::instance<DashboardController>();
    auto *meter = qobject_cast<Meter *>(d.item(QStringLiteral("Dashboard.meter.0")));
    d.check(QStringLiteral("qp5: each column has a meter"), meter != nullptr);
    if (!meter || !poller) return;
    d.check(QStringLiteral("qp5: a meter is a level indicator"), roleOf(meter) == QAccessible::ProgressBar);
    d.check(QStringLiteral("qp5: a meter follows the meter poll"), d.waitFor(2000, [&]() {
                const QVariantList p = poller->peaks();
                return !p.isEmpty() && std::abs(meter->level() - qBound(0.0, p.at(0).toReal(), 1.0)) < 1e-6;
            }));
    d.check(QStringLiteral("qp5: the bar spans the level of the width less the clip mark"),
            std::abs(meter->barWidth() - meter->level() * (meter->width() - Meter::kClipZone)) < 1e-6);
    QQuickItem *note = d.item(QStringLiteral("Dashboard.stale"));
    d.check(QStringLiteral("qp5: the meters are current"), note && !dc->stale() && !note->isVisible());

    // A standalone meter: levels are clamped
    Meter probe;
    probe.setLevel(1.7);
    const bool high = probe.level() == 1.0;
    probe.setLevel(-0.2);
    d.check(QStringLiteral("qp5: a meter's level is held to 0..1"), high && probe.level() == 0.0);

    // No reading for a while: stale, the meters keep their values and colour
    poller->setEnabled(poller->meterPoll(), false);
    d.waitIdle();
    d.spin(100);
    const qreal held = meter->level();
    const qreal opacity = meter->opacity();
    d.check(QStringLiteral("qp5: without readings the meters go stale"),
            d.waitFor(dc->staleAfterMs() + 1500, [&]() { return dc->stale(); }));
    d.check(QStringLiteral("qp5: the Overview shows the stale note"), note && note->isVisible());
    d.check(QStringLiteral("qp5: the note is a status for screen readers"), roleOf(note) == QAccessible::StatusBar);
    d.check(QStringLiteral("qp5: stale meters keep the last value"), meter->level() == held);
    d.check(QStringLiteral("qp5: stale meters are not greyed"), meter->opacity() == opacity);
    d.check(QStringLiteral("qp5: screenshot of the stale Overview"), d.grab(d.mainWindow(), QStringLiteral("window-dashboard-stale.png")));
    poller->setEnabled(poller->meterPoll(), true);
    d.check(QStringLiteral("qp5: a reading ends the stale state"), d.waitFor(2000, [&]() { return !dc->stale(); }));
    d.check(QStringLiteral("qp5: the note goes with it"), note && !note->isVisible());

    // Over a slow transport the threshold allows for 2 Hz meters
    poller->setSlowTransport(true);
    d.check(QStringLiteral("qp5: a slow transport waits longer before stale"), dc->staleAfterMs() >= 4 * Poller::kSlowIntervalMs);
    d.spin(1200);
    d.check(QStringLiteral("qp5: 2 Hz meters are not stale"), !dc->stale());
    poller->setSlowTransport(false);
    d.check(QStringLiteral("qp5: no clip mark without a clip"), !dc->clipped(0) || (poller->clipFlags() & 1u));
}

void checkNoDevice(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    DashboardController *dc = Registrar::instance<DashboardController>();
    QQuickItem *dash = d.item(QStringLiteral("Dashboard"));
    d.app().core->worker()->post([](FfiCore *c) { dspi_disconnect(c); });
    device->refresh();
    d.check(QStringLiteral("qp5: the device closes"), d.waitFor(3000, [&]() { return !device->connected(); }));
    d.check(QStringLiteral("qp5: without a device the cards fade out"),
            dash && d.waitFor(2000, [&]() { return dash->opacity() < 0.01; }) && !dash->isEnabled());
    d.check(QStringLiteral("qp5: without a device nothing is stale"), !dc->stale());
    d.spin(dc->staleAfterMs() + 300);
    d.check(QStringLiteral("qp5: and stays so"), !dc->stale() && dc->clipLatched() == 0);
    device->openDemo();
    d.check(QStringLiteral("qp5: the demo device opens again"), d.waitFor(10000, [&]() { return device->connected(); }));
    d.check(QStringLiteral("qp5: the cards fade back in"), dash && d.waitFor(2000, [&]() { return dash->opacity() > 0.99; }));
    d.waitIdle();
    d.check(QStringLiteral("qp5: the cards are back"), d.waitFor(2000, [&]() { return dc->cards().size() >= 2; }));
}

void dashboardSection(SmokeDriver &d) {
    d.waitIdle();
    checkTexts(d);
    checkCards(d);
    d.check(QStringLiteral("qp5: screenshot of the Overview"), d.grab(d.mainWindow(), QStringLiteral("window-dashboard.png")));
    checkDelay(d);
    checkLayout(d);
    checkMeters(d);
    checkNoDevice(d);
}

const SmokeDriver::Section dashboard("qp5-dashboard", 50, &dashboardSection);

} // namespace
