// WP QD2: the crossfeed window's pair map editor on the demo device, driven through
// the window's own functions, and its rows while a preset is selected.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "ExtController.h"

#include <QCoreApplication>
#include <QImage>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMouseEvent>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QThread>
#include <QVariant>
#include <QVariantMap>
#include <atomic>
#include <cstdio>

static void sendMouse(QQuickWindow *w, QEvent::Type type, const QPointF &local, Qt::MouseButton button = Qt::NoButton) {
    const Qt::MouseButtons held = type == QEvent::MouseButtonPress ? Qt::MouseButtons(Qt::LeftButton) : Qt::MouseButtons(Qt::NoButton);
    QMouseEvent e(type, local, local, QPointF(w->mapToGlobal(local.toPoint())), button, held, Qt::NoModifier);
    QCoreApplication::sendEvent(w, &e);
}

static QQuickItem *findItem(QQuickItem *in, const QString &name) {
    if (in->objectName() == name) return in;
    for (QQuickItem *child : in->childItems())
        if (QQuickItem *found = findItem(child, name)) return found;
    return nullptr;
}

static QVariantList pairsOf(std::initializer_list<std::pair<int, int>> list) {
    QVariantList pairs;
    for (const auto &p : list) {
        QVariantMap pair;
        pair.insert(QStringLiteral("left"), p.first);
        pair.insert(QStringLiteral("right"), p.second);
        pairs.append(pair);
    }
    return pairs;
}

void SmokeDriver::testPairMap() {
    section("QD2: crossfeed pair map");
    auto *b = m_.bridge;
    auto *ext = qobject_cast<ExtController *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("ext")).value<QObject *>());
    check("QD2: controller registered", ext != nullptr);
    QQuickWindow *win = nullptr;
    for (QQuickWindow *c : root()->findChildren<QQuickWindow *>())
        if (c->property("title").toString() == QLatin1String("Crossfeed")) win = c;
    check("QD2: window found", win != nullptr);
    if (!ext || !win) return;
    const auto item = [win](const char *name) { return win->findChild<QQuickItem *>(QString::fromLatin1(name)); };
    QQuickItem *controls = item("pairMapControls");
    QQuickItem *editor = item("pairMapEditor");
    QQuickItem *applyButton = item("pairMapApply");
    QQuickItem *warning = item("pairMapDraftWarning");
    QQuickItem *mapChips = item("pairMapChips");
    QQuickItem *plainChips = item("plainPairChips");
    QQuickItem *offer = item("k340MaskOffer");
    QQuickItem *params = item("crossfeedParams");
    QQuickItem *freqRow = item("crossfeedFreqRow");
    QQuickItem *feedRow = item("crossfeedFeedRow");
    check("QD2: items found", controls && editor && applyButton && warning && mapChips && plainChips && offer && params && freqRow && feedRow);
    if (!controls || !editor || !applyButton || !warning || !mapChips || !plainChips || !offer || !params || !freqRow || !feedRow) return;

    // The map the device reports, read past the controller
    const auto device = [b]() {
        QByteArray map(4, '\0');
        return dspi_fetch_crossfeed_pair_map(b->core(), reinterpret_cast<uint8_t *>(map.data()), 4) ? map.toHex() : QByteArray("failed");
    };
    const auto pick = [&](const char *key) {
        QMetaObject::invokeMethod(win, "pickPairChoice", Q_ARG(QVariant, QString::fromLatin1(key)));
    };
    // As a pick in the row's menu does
    const auto setOutput = [&](int pair, const char *side, int output) {
        const QString name = (QLatin1String(side) == QLatin1String("left") ? QStringLiteral("pairLeft%1") : QStringLiteral("pairRight%1")).arg(pair);
        QQuickItem *box = findItem(editor, name);
        check("QD2: the row's menu", box != nullptr);
        if (box) QMetaObject::invokeMethod(box, "activated", Q_ARG(int, output));
    };
    // The whole button shows, within the window's edges
    const auto applyInWindow = [&]() {
        const QRectF r = applyButton->mapRectToScene(QRectF(0, 0, applyButton->width(), applyButton->height()));
        return applyButton->isVisible() && r.top() >= 0 && r.left() >= 0 && r.bottom() <= win->height() && r.right() <= win->width();
    };
    // A real click on the real button: true when it landed where the button is
    const auto clickApply = [&]() {
        if (!applyInWindow()) return false;
        const QPointF at = applyButton->mapToScene(QPointF(applyButton->width() / 2, applyButton->height() / 2));
        sendMouse(win, QEvent::MouseMove, at);
        sendMouse(win, QEvent::MouseButtonPress, at, Qt::LeftButton);
        sendMouse(win, QEvent::MouseButtonRelease, at, Qt::LeftButton);
        return true;
    };
    const auto choice = [ext](const char *want) { return ext->choice() == QLatin1String(want); };

    win->show();
    spin(100);
    check("QD2: the window watches while open", ext->watching());
    check("QD2: the demo device has the map", ext->hasPairMap() && controls->isVisible());
    check("QD2: map chips, not the plain ones", mapChips->isVisible() && !plainChips->isVisible());
    printf("smoke: QD2 pairs: %d, map %s\n", ext->pairCount(), device().constData());
    check("QD2: four pairs on the demo device", ext->pairCount() == 4);

    // The presets
    pick("k340");
    check("QD2: K340: shown", waitFor(1000, [&]() { return choice("k340"); }));
    check("QD2: K340: on the device", device() == "02130000");
    const QVariantList k340 = ext->pairs();
    check("QD2: K340: pairs 3/1 and 4/2",
          k340.size() == 4 && k340.at(0) == pairsOf({ { 2, 0 } }).at(0) && k340.at(1) == pairsOf({ { 3, 1 } }).at(0)
              && k340.at(2) == pairsOf({ { 4, 5 } }).at(0));
    b->setCrossfeedOutputPairMask(1);
    spin(50);
    check("QD2: K340: the mask is offered", offer->isVisible());
    check("QD2: K340: the mask is not changed unasked", b->crossfeedOutputPairMask() == 1);
    QMetaObject::invokeMethod(win, "enableK340Mask");
    spin(50);
    check("QD2: K340: the offer enables both pairs", b->crossfeedOutputPairMask() == 3 && !offer->isVisible());
    win->grabWindow().save(m_out + QStringLiteral("/window-crossfeed-k340.png"));

    pick("default");
    check("QD2: Default: shown", waitFor(1000, [&]() { return choice("default"); }));
    check("QD2: Default: on the device", device() == "00000000");

    // Custom: edit, apply, read back. Opened from the Outputs menu.
    QObject *menu = win->findChild<QObject *>(QStringLiteral("pairMapMenu"));
    check("QD2: the Outputs menu", menu != nullptr);
    if (menu) QMetaObject::invokeMethod(menu, "activate", Q_ARG(QVariant, 2));
    spin(50);
    check("QD2: Custom opens the editor", editor->isVisible());
    check("QD2: the editor starts from the device's map", win->property("draftValid").toBool() && !warning->isVisible());
    // -- WP QD40 -- (QD23-1) The menus show "Out N": sized for that, not for the
    // longest option, so in English they keep their 92 px and stay on the card
    {
        const QRectF card = editor->mapRectToScene(QRectF(0, 0, editor->width(), editor->height()));
        bool inside = true, plain = true;
        for (int p = 0; p < ext->pairCount(); ++p)
            for (const QString &name : { QStringLiteral("pairLeft%1").arg(p), QStringLiteral("pairRight%1").arg(p) }) {
                QQuickItem *box = findItem(editor, name);
                if (!box) { inside = plain = false; continue; }
                inside = inside && card.contains(box->mapRectToScene(QRectF(0, 0, box->width(), box->height())));
                plain = plain && qAbs(box->width() - 92) < 0.5;
            }
        check("QD40: the pair menus stay on the editor card", inside);
        check("QD40: the pair menus keep their English width", plain);
    }
    setOutput(0, "left", 7);
    setOutput(3, "right", 0);
    spin(50);
    check("QD2: a valid draft can be applied", win->property("draftValid").toBool() && applyButton->isEnabled());
    check("QD2: Apply is inside the window", waitFor(1500, applyInWindow));
    spin(400);      // the refit and the scroll settled
    printf("smoke: QD2 editor open: window %d of fit %d\n", win->height(), int(win->property("fitHeight").toDouble()));
    check("QD2: Apply stays inside the window", applyInWindow());
    win->grabWindow().save(m_out + QStringLiteral("/window-crossfeed-editor.png"));
    // A window the editor doesn't fit: scrolled to Apply, which is how a short screen goes
    const int fullHeight = win->height();
    win->setHeight(win->minimumHeight());
    spin(100);
    check("QD2: a short window hides Apply", !applyInWindow());
    QMetaObject::invokeMethod(win, "revealPairEditor");
    spin(100);
    check("QD2: scrolled to Apply in a short window", applyInWindow());
    win->grabWindow().save(m_out + QStringLiteral("/window-crossfeed-editor-short.png"));
    check("QD2: the Apply button takes a click", clickApply());
    check("QD2: Custom: the editor closes", waitFor(1000, [&]() { return !editor->isVisible(); }));
    printf("smoke: QD2 custom map on the device: %s\n", device().constData());
    check("QD2: Custom: read back", device() == "17000006");
    win->setHeight(fullHeight);
    check("QD2: Custom: shown as custom", choice("custom") && ext->lastError().isEmpty());
    const QVariantList custom = ext->pairs();
    check("QD2: Custom: pairs shown", custom.size() == 4 && custom.at(0) == pairsOf({ { 7, 1 } }).at(0) && custom.at(3) == pairsOf({ { 6, 0 } }).at(0));

    // An invalid pair is refused: the editor says so, and nothing reaches the device
    const QByteArray before = device();
    pick("custom");
    spin(50);
    setOutput(1, "left", 3);    // pair 2 on one output twice
    spin(50);
    check("QD2: the same output twice: told at once", !win->property("draftValid").toBool() && warning->isVisible() && !applyButton->isEnabled());
    spin(100);
    check("QD2: the same output twice: Apply is in the window, off", applyInWindow() && !applyButton->isEnabled());
    clickApply();
    spin(200);
    check("QD2: the same output twice: the click does nothing", editor->isVisible() && !ext->busy());
    QMetaObject::invokeMethod(win, "applyDraft");
    spin(200);
    check("QD2: the same output twice: refused, editor open", editor->isVisible() && !ext->lastError().isEmpty());
    check("QD2: the same output twice: device unchanged", device() == before);
    QMetaObject::invokeMethod(win, "cancelDraft");
    spin(50);
    check("QD2: Cancel closes the editor and the error", !editor->isVisible() && ext->lastError().isEmpty());

    const struct { const char *what; QVariantList pairs; } refused[] = {
        { "an output in two pairs", pairsOf({ { 0, 1 }, { 1, 2 }, { 4, 5 }, { 6, 7 } }) },
        { "(0, 0), which would encode as the default pair", pairsOf({ { 0, 0 }, { 2, 3 }, { 4, 5 }, { 6, 7 } }) },
        { "the PDM sub", pairsOf({ { 8, 1 }, { 2, 3 }, { 4, 5 }, { 6, 7 } }) },
        { "too few pairs", pairsOf({ { 0, 1 } }) },
        { "a negative output", pairsOf({ { -1, 1 }, { 2, 3 }, { 4, 5 }, { 6, 7 } }) },
    };
    for (const auto &r : refused) {
        check(r.what, !ext->pairsValid(r.pairs));
        check(r.what, !ext->applyPairs(r.pairs));
        spin(20);
        check(r.what, device() == before && !ext->busy());
    }
    ext->clearError();
    check("QD2: an unknown preset is refused", !ext->applyPreset(QStringLiteral("nope")));

    // The editor's rule is the core's: same answers on maps, valid and not
    const QByteArray maps[] = {
        QByteArray::fromHex("00000000"), QByteArray::fromHex("02130000"), QByteArray::fromHex("76543210"),
        QByteArray::fromHex("10325476"), QByteArray::fromHex("10000000"), QByteArray::fromHex("11000000"),
        QByteArray::fromHex("02000000"), QByteArray::fromHex("80000000"), QByteArray::fromHex("08000000"),
        QByteArray::fromHex("02020000"), QByteArray::fromHex("20023000"), QByteArray::fromHex("00000010"),
        QByteArray::fromHex("54000000"), QByteArray::fromHex("00FF0000"),
    };
    for (const QByteArray &map : maps) {
        // Through the pairs, as the editor has them; 0 is the default pair
        QVariantList pairs;
        for (int p = 0; p < 4; p++) {
            const auto byte = uint8_t(map.at(p));
            QVariantMap pair;
            pair.insert(QStringLiteral("left"), byte == 0 ? 2 * p : byte & 0x0F);
            pair.insert(QStringLiteral("right"), byte == 0 ? 2 * p + 1 : byte >> 4);
            pairs.append(pair);
        }
        const bool editorSays = ext->pairsValid(pairs);
        const bool coreSays = dspi_set_crossfeed_pair_map(b->core(), reinterpret_cast<const uint8_t *>(map.constData()), 4) == 0;
        if (editorSays != coreSays) printf("smoke: QD2 disagree on %s: editor %d, core %d\n", map.toHex().constData(), editorSays, coreSays);
        check("QD2: the editor and the core agree", editorSays == coreSays);
    }

    // A preset load changes the map with no write from here
    pick("k340");
    check("QD2: K340 to save", waitFor(1000, [&]() { return choice("k340") && !ext->busy(); }));
    const int slot = 7;
    check("QD2: save", b->savePreset(slot) == 0);
    pick("default");
    check("QD2: map cleared", waitFor(1000, [&]() { return choice("default") && !ext->busy(); }));
    check("QD2: load", b->loadPreset(slot) == 0);
    check("QD2: a preset load shows its map", waitFor(1500, [&]() { return choice("k340"); }));
    b->deletePreset(slot);
    spin(100);

    pick("default");
    check("QD2: back to default", waitFor(1000, [&]() { return choice("default"); }));

    // Hidden on a stall: what stock firmware's connect probe leaves
    pick("custom");
    spin(50);
    ext->setStatusOverride(R"({"ok":true,"supported":[],"hasPairMap":false,"hasCoreVreg":false})");
    spin(50);
    check("QD2: a stall hides the pair map", !ext->hasPairMap() && !controls->isVisible());
    check("QD2: a stall closes the editor", !win->property("editingPairs").toBool());
    check("QD2: a stall leaves the plain chips", plainChips->isVisible() && !mapChips->isVisible());
    check("QD2: a stall refuses a write", !ext->applyPreset(QStringLiteral("k340")));
    win->grabWindow().save(m_out + QStringLiteral("/window-crossfeed-stock.png"));
    ext->setStatusOverride(QByteArray());
    spin(50);
    check("QD2: the map comes back", ext->hasPairMap() && controls->isVisible());

    // A new device starts from its own map, not the last one's
    pick("k340");
    check("QD2: K340 again", waitFor(1000, [&]() { return choice("k340") && !ext->busy(); }));
    b->disconnectDevice();
    // At once: a hotplug poll soon reselects the demo device
    check("QD2: no device: hidden", !ext->hasPairMap() && !controls->isVisible());
    b->rescan();
    check("QD2: connected again", b->connected());
    check("QD2: the new device's map", waitFor(1000, [&]() { return ext->hasPairMap() && ext->choice() == QLatin1String("default") && controls->isVisible(); }) && device() == "00000000");

    // An Apply behind a slow job, then a change of device: it never reaches the new one
    std::atomic<bool> gate { false };
    std::atomic<int> reached { 0 };
    ext->setWriteHook([&]() {
        reached++;
        while (!gate) QThread::msleep(2);
        return true;
    });
    check("QD2: an Apply holds on the worker", ext->applyPreset(QStringLiteral("k340")) && waitFor(2000, [&]() { return reached.load() == 1; }));
    check("QD2: a second Apply queues behind it", ext->applyPreset(QStringLiteral("k340")) && ext->busy());
    b->disconnectDevice();
    b->rescan();
    check("QD2: a fresh device while they wait", b->connected() && device() == "00000000");
    gate = true;
    check("QD2: both Applies end", waitFor(2000, [&]() { return !ext->busy(); }));
    spin(100);
    check("QD2: both ran past the hold", reached.load() == 2);
    check("QD2: no write reached the new device", device() == "00000000" && ext->choice() == QLatin1String("default"));
    ext->setWriteHook(nullptr);

    // The dimmed rows: a preset's own values from the window's table, Custom's live
    const auto opacity = [params]() { return params->opacity(); };
    const auto rows = [&]() { return qMakePair(freqRow->property("displayValue").toDouble(), feedRow->property("displayValue").toDouble()); };
    const auto rowsAre = [&](double freq, double feed) { return qAbs(rows().first - freq) < 0.01 && qAbs(rows().second - feed) < 0.01; };
    b->setCrossfeedPreset(3);
    b->setCrossfeedFreq(1500.0f);
    b->setCrossfeedFeed(12.0f);
    spin(50);
    check("QD2: Custom: rows not dimmed, the device's values", opacity() == 1.0 && rowsAre(1500, 12));
    b->setCrossfeedPreset(0);
    spin(50);
    check("QD2: Default: rows dimmed, its own values", opacity() < 1.0 && rowsAre(700, 4.5));
    b->setCrossfeedPreset(1);
    spin(50);
    check("QD2: Chu Moy: rows dimmed, its own values", opacity() < 1.0 && rowsAre(700, 6.0));
    b->setCrossfeedPreset(2);
    spin(50);
    check("QD2: Jan Meier: rows dimmed, its own values", opacity() < 1.0 && rowsAre(650, 9.5));
    check("QD2: the device's custom values stay", qAbs(b->crossfeedFreq() - 1500) < 0.01 && qAbs(b->crossfeedFeed() - 12) < 0.01);
    b->setCrossfeedPreset(0);

    win->setVisible(false);
    spin(50);
    check("QD2: stops watching when closed", !ext->watching());
}

// -- WP QD3 --
// Core voltage on Settings > Overview: the status as the device has it, live while the
// page shows and silent otherwise, and nothing of it on firmware without the extension.
void SmokeDriver::testCoreVoltage() {
    section("QD3: core voltage status");
    auto *b = m_.bridge;
    auto *ext = qobject_cast<ExtController *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("ext")).value<QObject *>());
    QQuickWindow *settings = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
    check("QD3: controller and window", ext && settings);
    if (!ext || !settings) return;
    const auto navigate = [&](const char *id) {
        QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QString::fromLatin1(id)));
        spin(100);
    };
    // Looked up each time: the page is built anew on each visit
    const auto item = [&](const char *name) { return findItem(settings->contentItem(), QString::fromLatin1(name)); };
    const auto shows = [&](const char *name) { QQuickItem *i = item(name); return i && i->isVisible(); };
    const auto row = [&](const char *name) { QQuickItem *i = item(name); return i ? i->property("value").toString() : QStringLiteral("<no row>"); };
    // What the device says, read past the controller
    const auto device = [b]() {
        QByteArray reply(512, '\0');
        const uint32_t n = dspi_fetch_core_vreg(b->core(), reply.data(), uint32_t(reply.size()));
        reply.truncate(n > 0 ? int(n) - 1 : 0);
        return QJsonDocument::fromJson(reply).object();
    };
    const auto waiting = [&]() { return row("coreVregRunning") == QStringLiteral("Reading…") && !shows("coreVregStored"); };
    const auto refused = [&]() { return row("coreVregRunning") == QStringLiteral("The device did not answer") && !shows("coreVregStored"); };
    const auto mv = [](int value) { return QString::number(value / 1000.0, 'f', 2) + QStringLiteral(" V"); };
    const auto rowShowsDevice = [&]() {
        const QJsonObject d = device();
        return row("coreVregRunning") == mv(d.value(QStringLiteral("millivolts")).toInt())
            && row("coreVregHangs") == QString::number(d.value(QStringLiteral("hangCount")).toInt());
    };

    settings->setVisible(true);
    navigate("overview");
    check("QD3: the demo device lists it", ext->hasCoreVreg());
    check("QD3: watching while the page shows", ext->coreVregWatching());
    check("QD3: the section shows", shows("coreVoltageSection"));
    check("QD3: the first read arrives", waitFor(2000, [&]() { return shows("coreVregStored") && row("coreVregRunning").endsWith(QLatin1String(" V")); }));
    const QJsonObject first = device();
    printf("smoke: QD3 running at %d mV, row %s, stored %d, default %d\n", first.value(QStringLiteral("millivolts")).toInt(),
           qPrintable(row("coreVregRunning")), first.value(QStringLiteral("storedSel")).toInt(), first.value(QStringLiteral("defaultVreg")).toInt());
    check("QD3: live mV and hang count are the device's", first.value(QStringLiteral("millivolts")).toInt() > 0 && rowShowsDevice());
    check("QD3: no hangs yet", row("coreVregHangs") == QLatin1String("0"));
    check("QD3: the other rows", row("coreVregStored") == QLatin1String("Build default") && row("coreVregDefault") == mv(first.value(QStringLiteral("millivolts")).toInt())
                                     && row("coreVregAuto") == QLatin1String("On") && row("coreVregFailedTrial") == QLatin1String("None")
                                     && !shows("coreVregTrial") && !row("coreVregUptime").isEmpty());
    settings->grabWindow().save(m_out + QStringLiteral("/settings-overview-core-voltage.png"));

    // Live: the device's uptime and hang count move, and the rows follow unasked
    check("QD3: time passing on the device", dspi_core_vreg_demo_event(b->core(), 3, 3700) == 0);
    check("QD3: uptime follows", waitFor(2500, [&]() { return row("coreVregUptime").startsWith(QLatin1String("1 h")); }));
    check("QD3: a hang", dspi_core_vreg_demo_event(b->core(), 0, 0) == 0);
    check("QD3: the hang count follows", waitFor(2500, [&]() { return row("coreVregHangs") == QLatin1String("1"); }));
    check("QD3: a second hang", dspi_core_vreg_demo_event(b->core(), 0, 0) == 0);
    check("QD3: the raise follows", waitFor(2500, [&]() { return row("coreVregStored").endsWith(QLatin1String(", raised by the device")) && rowShowsDevice(); }));
    printf("smoke: QD3 after two hangs: running %s, stored %s, hangs %s\n", qPrintable(row("coreVregRunning")), qPrintable(row("coreVregStored")), qPrintable(row("coreVregHangs")));
    settings->grabWindow().save(m_out + QStringLiteral("/settings-overview-core-voltage-raised.png"));

    // Reads stop with the window shut and with the page left
    settings->setVisible(false);
    spin(100);
    check("QD3: a shut window stops the reads", !ext->coreVregWatching());
    const QVariantMap frozen = ext->coreVreg();
    dspi_core_vreg_demo_event(b->core(), 3, 100);
    spin(1500);
    check("QD3: nothing read while shut", ext->coreVreg() == frozen);
    settings->setVisible(true);
    check("QD3: reopening reads at once", waitFor(800, [&]() { return ext->coreVreg() != frozen; }) && ext->coreVregWatching());
    navigate("about");
    check("QD3: leaving the page stops the reads", !ext->coreVregWatching());
    const QVariantMap left = ext->coreVreg();
    dspi_core_vreg_demo_event(b->core(), 3, 100);
    spin(1500);
    check("QD3: nothing read off the page", ext->coreVreg() == left);
    navigate("overview");
    check("QD3: back on the page", ext->coreVregWatching() && waitFor(800, [&]() { return ext->coreVreg() != left; }));

    // A failed read shows no old values as if they were live; the next good one mends it
    ext->setCoreVregOverride(R"({"ok":false,"code":1,"message":"stalled"})");
    check("QD3: a failed read says so", waitFor(2500, refused) && ext->coreVregFailed());
    ext->setCoreVregOverride(QByteArray());
    check("QD3: and recovers", waitFor(2500, [&]() { return shows("coreVregStored") && !ext->coreVregFailed(); }));

    // Reads stay off the GUI thread: with the worker held the window still runs, and a
    // change of device shows nothing of the old one, then the new one's own status
    check("QD3: the old device was raised", row("coreVregStored") != QLatin1String("Build default"));
    std::atomic<bool> gate { false };
    std::atomic<bool> held { false };
    ext->setWriteHook([&]() {
        held = true;
        while (!gate) QThread::msleep(2);
        return true;
    });
    check("QD3: the worker held", ext->applyPreset(QStringLiteral("default")) && waitFor(2000, [&]() { return held.load(); }));
    spin(1200);     // a poll comes due behind the hold, and the GUI thread runs on
    b->disconnectDevice();
    check("QD3: no device: nothing kept", !ext->hasCoreVreg() && !shows("coreVoltageSection") && ext->coreVreg().isEmpty());
    b->rescan();
    check("QD3: a fresh device", b->connected());
    navigate("overview");   // the window leaves the page when the device goes
    check("QD3: nothing of the old device while the read waits", shows("coreVoltageSection") && waiting());
    gate = true;
    check("QD3: the fresh device's status", waitFor(2500, [&]() { return shows("coreVregStored") && row("coreVregStored") == QLatin1String("Build default") && row("coreVregHangs") == QLatin1String("0"); }));
    check("QD3: the write behind the hold ended", waitFor(1000, [&]() { return !ext->busy(); }));
    ext->setWriteHook(nullptr);

    // Stock firmware's connect probe finds nothing: no section, no reads
    ext->setStatusOverride(R"({"ok":true,"supported":[],"hasPairMap":false,"hasCoreVreg":false})");
    spin(50);
    check("QD3: stock: hidden", !ext->hasCoreVreg() && !shows("coreVoltageSection") && ext->coreVreg().isEmpty());
    check("QD3: stock: the page stays", b->connected() && item("coreVoltageSection") != nullptr);
    settings->grabWindow().save(m_out + QStringLiteral("/settings-overview-stock.png"));
    spin(1500);
    check("QD3: stock: no reads", ext->coreVreg().isEmpty() && !ext->coreVregFailed());
    ext->setStatusOverride(QByteArray());
    check("QD3: extension back: shown and read", waitFor(2500, [&]() { return ext->hasCoreVreg() && shows("coreVregStored"); }));

    settings->setVisible(false);
    spin(50);
}
