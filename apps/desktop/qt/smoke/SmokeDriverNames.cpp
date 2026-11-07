// WP QD4: source and device names round trip through the real rows on the demo
// device, and another client's rename reaches them.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "NamesController.h"

#include <QCoreApplication>
#include <QKeyEvent>
#include <QPointer>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QThread>
#include <atomic>
#include <cstdio>

namespace {
QQuickItem *findItem(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = findItem(c, name)) return found;
    return nullptr;
}

void pressKey(QQuickWindow *w, int key, Qt::KeyboardModifiers mods, const QString &text) {
    QKeyEvent press(QEvent::KeyPress, key, mods, text);
    QCoreApplication::sendEvent(w, &press);
    QKeyEvent release(QEvent::KeyRelease, key, mods, text);
    QCoreApplication::sendEvent(w, &release);
}

// Replace the field's text with keys, as a user would
void typeInto(QQuickWindow *w, QQuickItem *field, const QString &text) {
    field->forceActiveFocus();
    QMetaObject::invokeMethod(field, "selectAll");
    pressKey(w, Qt::Key_Backspace, Qt::NoModifier, QString());
    for (const QChar c : text) pressKey(w, Qt::Key_unknown, Qt::NoModifier, QString(c));
}

QString textOf(QQuickItem *field) { return field->property("text").toString(); }
}

void SmokeDriver::testNames() {
    section("names: rows on the demo device (QD4)");
    auto *b = m_.bridge;
    FfiCore *core = b->core();
    auto *names = qobject_cast<NamesController *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("deviceNames")).value<QObject *>());
    QQuickWindow *settings = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
    QObject *ctx = settings ? settings->findChild<QObject *>(QStringLiteral("settingsContext")) : nullptr;
    QObject *reason = settings ? settings->findChild<QObject *>(QStringLiteral("saveBarReason")) : nullptr;
    check("QD4: the controller, the window and its context", names && settings && ctx && reason);
    if (!names || !settings || !ctx || !reason) return;
    check("QD4: the demo device has both kinds of name", names->sourceNamesSupported() && names->deviceNameSupported());

    // Read over the wire: what the device holds, not the cache
    const auto liveDevice = [core]() {
        char buf[32] = {};
        return dspi_get_device_name(core, buf, 32) ? QString::fromUtf8(buf) : QStringLiteral("<failed>");
    };
    const auto liveSource = [core](int s) {
        char buf[32] = {};
        return dspi_get_input_source_name(core, uint8_t(s), buf, 32) ? QString::fromUtf8(buf) : QStringLiteral("<failed>");
    };
    const auto save = [&]() {
        QMetaObject::invokeMethod(ctx, "save");
        return waitFor(2000, [names]() { return !names->saving(); });
    };
    const auto dirty = [ctx]() { return ctx->property("dirty").toBool(); };
    const auto page = [&](const char *id, const char *field) -> QQuickItem * {
        QMetaObject::invokeMethod(settings, "navigate", Q_ARG(QVariant, QString::fromLatin1(id)));
        spin(300);
        return findItem(settings->contentItem(), QString::fromLatin1(field));
    };

    std::atomic<int> writes { 0 };
    std::atomic<bool> offUiThread { true };
    QThread *const ui = QThread::currentThread();
    names->setWriteHook([&]() {
        writes++;
        if (QThread::currentThread() == ui) offUiThread = false;
        return true;
    });

    settings->setVisible(true);
    spin(300);

    // ── Device name ──
    QQuickItem *device = page("global", "deviceNameField");
    check("QD4: the Device Name row is on Global Parameters", device && device->isVisible());
    if (!device) { names->setWriteHook(nullptr); return; }
    const QString firstName = liveDevice();
    printf("smoke: QD4 demo device name: %s\n", qPrintable(firstName));
    check("QD4: the row shows the device's name", textOf(device) == firstName && names->deviceName() == firstName);

    typeInto(settings, device, QString::fromUtf8("Den \xc3\xa9x"));
    check("QD4: the field has the keyboard", device->hasActiveFocus());
    printf("smoke: QD4 device field reads: %s\n", qPrintable(textOf(device)));
    check("QD4: what the device can't store is dropped as typed", textOf(device) == QLatin1String("Den x"));
    check("QD4: a draft shows the save bar", dirty() && names->devicePending());
    check("QD4: nothing is sent before Save", liveDevice() == firstName && writes == 0);
    settings->grabWindow().save(m_out + QStringLiteral("/settings-names-global.png"));
    check("QD4: save finishes", save());
    check("QD4: a device name round trips", liveDevice() == QLatin1String("Den x") && names->deviceName() == QLatin1String("Den x"));
    check("QD4: one write, off the UI thread", writes == 1 && offUiThread);
    check("QD4: nothing is left to save", !dirty() && !names->devicePending() && textOf(device) == QLatin1String("Den x"));

    typeInto(settings, device, QStringLiteral("Den x "));
    check("QD4: a trailing space is no change", !dirty() && textOf(device) == QLatin1String("Den x "));
    typeInto(settings, device, QString(40, QLatin1Char('a')));
    check("QD4: 31 characters at most", textOf(device) == QString(31, QLatin1Char('a')));
    check("QD4: save finishes (longest)", save());
    check("QD4: the longest name round trips", liveDevice() == QString(31, QLatin1Char('a')));
    typeInto(settings, device, QString());
    check("QD4: emptied is a change", dirty());
    check("QD4: save finishes (empty)", save());
    check("QD4: an empty name clears it", liveDevice().isEmpty() && names->deviceName().isEmpty());

    // ── Another client, on the device name ──
    const QByteArray shed("Shed"), other("Other");
    check("QD4: rename elsewhere is accepted", dspi_demo_rename_elsewhere(core, 0xFF, shed.constData()));
    check("QD4: another client's rename reaches the row", waitFor(2000, [&]() { return textOf(device) == QLatin1String("Shed"); }));
    check("QD4: and leaves nothing to save", !dirty() && names->deviceName() == QLatin1String("Shed"));
    typeInto(settings, device, QStringLiteral("Mine"));
    check("QD4: rename elsewhere over a draft", dspi_demo_rename_elsewhere(core, 0xFF, other.constData()));
    check("QD4: the stored name follows", waitFor(2000, [&]() { return names->deviceName() == QLatin1String("Other"); }));
    check("QD4: and the draft stays", textOf(device) == QLatin1String("Mine") && dirty());
    QMetaObject::invokeMethod(ctx, "revert");
    spin(100);
    check("QD4: revert shows the stored name", textOf(device) == QLatin1String("Other") && !dirty());

    // ── Source names ──
    QPointer<QQuickItem> i2s = page("inputs", "sourceNameField2");
    QQuickItem *usb = findItem(settings->contentItem(), QStringLiteral("sourceNameField0"));
    check("QD4: source rows are on Inputs", i2s && usb && i2s->isVisible() && usb->isVisible());
    if (!i2s || !usb) { names->setWriteHook(nullptr); return; }
    for (const QVariant &v : b->inputSources()) {
        const int id = v.toMap().value("id").toInt();
        check("QD4: a row for each source", findItem(settings->contentItem(), QStringLiteral("sourceNameField%1").arg(id)) != nullptr);
    }
    writes = 0;
    typeInto(settings, usb, QStringLiteral("Mac"));
    typeInto(settings, i2s, QStringLiteral("Tap"));
    b->setMasterVolume(-21.0f);                 // the state changes while typing
    spin(200);
    check("QD4: a state change leaves the row being typed in", i2s && i2s->hasActiveFocus() && textOf(i2s) == QLatin1String("Tap"));
    if (!i2s) { names->setWriteHook(nullptr); return; }
    typeInto(settings, i2s, QStringLiteral("Tape"));
    check("QD4: nothing is sent before Save (sources)", liveSource(2).isEmpty() && liveSource(0).isEmpty() && writes == 0);
    const QSize size = settings->size();
    settings->resize(size.width(), 1200);       // the section is at the page's end
    spin(100);
    settings->grabWindow().save(m_out + QStringLiteral("/settings-names-inputs.png"));
    settings->resize(size);
    check("QD4: save finishes (sources)", save());
    check("QD4: source names round trip", liveSource(2) == QLatin1String("Tape") && liveSource(0) == QLatin1String("Mac"));
    check("QD4: one write each", writes == 2 && offUiThread && !dirty());

    typeInto(settings, i2s, QStringLiteral("Other"));
    check("QD4: a source draft is dirty", dirty());
    QMetaObject::invokeMethod(ctx, "revert");
    spin(100);
    check("QD4: revert drops it, unsent", textOf(i2s) == QLatin1String("Tape") && !dirty() && liveSource(2) == QLatin1String("Tape"));

    // ── Another client, on a source ──
    const QByteArray deck("Deck"), none("");
    check("QD4: source rename elsewhere is accepted", dspi_demo_rename_elsewhere(core, 2, deck.constData()));
    check("QD4: another client's source rename reaches the row", waitFor(2000, [&]() { return textOf(i2s) == QLatin1String("Deck"); }));
    check("QD4: source rename elsewhere, cleared", dspi_demo_rename_elsewhere(core, 2, none.constData()));
    check("QD4: another client's clear reaches the row", waitFor(2000, [&]() { return textOf(i2s).isEmpty(); }));

    // ── A refused write ──
    names->setWriteHook([]() { return false; });
    typeInto(settings, usb, QStringLiteral("Refused"));
    check("QD4: save finishes (refused)", save());
    spin(100);
    const QString shown = reason->property("text").toString();
    printf("smoke: QD4 save bar shows: %s\n", qPrintable(shown));
    check("QD4: a refusal shows in the save bar", reason->property("visible").toBool() && shown.startsWith(QLatin1String("Not saved: ")));
    check("QD4: the draft stays", dirty() && textOf(usb) == QLatin1String("Refused") && liveSource(0) == QLatin1String("Mac"));
    names->setWriteHook(nullptr);
    check("QD4: save finishes (after a refusal)", save());
    check("QD4: a second try goes through", liveSource(0) == QLatin1String("Refused") && !dirty());

    // ── A change of device during a save ──
    std::atomic<bool> gate { false };
    std::atomic<bool> reached { false };
    names->setWriteHook([&]() {
        reached = true;
        while (!gate) QThread::msleep(2);
        return true;
    });
    typeInto(settings, usb, QStringLiteral("OldDevice"));
    QMetaObject::invokeMethod(ctx, "save");
    check("QD4: the save holds on the worker, the UI runs", waitFor(2000, [&]() { return reached.load(); }));
    b->disconnectDevice();
    printf("smoke: QD4 no device: connected %d, sources %d, device %d, row visible %d\n", b->connected(),
           names->sourceNamesSupported(), names->deviceNameSupported(), usb->isVisible());
    check("QD4: no device: no draft", !dirty());
    check("QD4: no device: no names", !names->sourceNamesSupported() && !names->deviceNameSupported());
    check("QD4: no device: no section", !usb->isVisible());
    b->rescan();
    check("QD4: back on a fresh demo device", b->connected() && liveSource(0).isEmpty());
    gate = true;
    check("QD4: save ends", waitFor(2000, [names]() { return !names->saving(); }));
    spin(100);
    check("QD4: the old device's name did not reach the new one", liveSource(0).isEmpty() && !dirty());
    names->setWriteHook(nullptr);
    settings->setVisible(false);
}
