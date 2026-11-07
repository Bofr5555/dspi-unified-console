// WP QD2: the crossfeed window's pair map editor on the demo device, driven through
// the window's own functions, and its rows while a preset is selected.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "ExtController.h"

#include <QImage>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QVariant>
#include <QVariantMap>
#include <cstdio>

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
    setOutput(0, "left", 7);
    setOutput(3, "right", 0);
    spin(50);
    check("QD2: a valid draft can be applied", win->property("draftValid").toBool() && applyButton->isEnabled());
    win->grabWindow().save(m_out + QStringLiteral("/window-crossfeed-editor.png"));
    QMetaObject::invokeMethod(win, "applyDraft");
    check("QD2: Custom: the editor closes", waitFor(1000, [&]() { return !editor->isVisible(); }));
    printf("smoke: QD2 custom map on the device: %s\n", device().constData());
    check("QD2: Custom: read back", device() == "17000006");
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
