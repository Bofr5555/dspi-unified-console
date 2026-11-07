// WP QD31: a bridge setter that fails in SettingsContext.save() doesn't read as
// saved, on the real settings window and the demo device. The device's own
// refusals are what the core rejects (a mode out of range), the demo's output
// config save told how to answer, and a DAC mute read-back that differs from what
// was sent.
#include "SmokeDriver.h"

#include "DSPiBridge.h"

#include <QQuickWindow>
#include <QVariant>
#include <cstdio>

void SmokeDriver::testSaveBarSetters() {
    section("settings: a failed setter on Save (QD31)");
    DSPiBridge *b = m_.bridge;
    QQuickWindow *settings = nullptr;
    for (QQuickWindow *w : root()->findChildren<QQuickWindow *>())
        if (w->property("currentPage").isValid() && w->metaObject()->indexOfMethod("navigate(QVariant)") >= 0) settings = w;
    QObject *ctx = settings ? settings->findChild<QObject *>(QStringLiteral("settingsContext")) : nullptr;
    QObject *bar = settings ? settings->findChild<QObject *>(QStringLiteral("saveBarReason")) : nullptr;
    check("QD31: the window, its context and the reason's text", ctx && bar);
    if (!ctx || !bar) return;
    if (!b->connected()) b->rescan();
    check("QD31: on the demo", b->connected());
    settings->setVisible(true);

    const auto edit = [ctx](const char *name, const QVariant &value) {
        QMetaObject::invokeMethod(ctx, "edit", Q_ARG(QVariant, QString::fromLatin1(name)), Q_ARG(QVariant, value));
    };
    const auto save = [ctx]() { QMetaObject::invokeMethod(ctx, "save"); };
    const auto revert = [ctx]() { QMetaObject::invokeMethod(ctx, "revert"); };
    const auto dirty = [ctx]() { return ctx->property("dirty").toBool(); };
    const auto edited = [ctx]() { return ctx->property("edited").toBool(); };
    const auto reason = [ctx]() { return ctx->property("saveError").toString(); };
    // On the bar, as the context has it, and it says what wasn't saved
    const auto barSays = [&](const char *what) {
        const QString shown = bar->property("text").toString();
        printf("smoke: QD31 save bar shows: %s\n", qPrintable(shown));
        // The bar slides in
        return waitFor(1000, [bar]() { return bar->property("visible").toBool(); }) && shown == reason() && shown.startsWith(QLatin1String("Not saved: "))
            && shown.contains(QLatin1String(what));
    };
    const auto clean = [&]() { return !dirty() && !edited() && reason().isEmpty(); };
    const int startup = b->presetStartupMode(), master = b->masterVolumeMode(), output = b->outputConfigMode();
    check("QD31: nothing to save at first", clean());

    // ── The first step fails: nothing after it goes out ──
    edit("startupMode", 7);
    edit("masterVolumeMode", master ? 0 : 1);
    save();
    check("QD31: startup preset: the bar says so", barSays("startup preset"));
    check("QD31: startup preset: still dirty, still edited", dirty() && edited());
    check("QD31: startup preset: the device has the old one", b->presetStartupMode() == startup);
    check("QD31: startup preset: the later step never went out", b->masterVolumeMode() == master);
    spin(100);
    check("QD31: startup preset: the draft stays", ctx->property("startupMode").toInt() == 7 && !reason().isEmpty());
    settings->grabWindow().save(m_out + QStringLiteral("/settings-save-setter-refused.png"));
    revert();
    check("QD31: revert drops the draft and the reason", clean() && ctx->property("startupMode").toInt() == startup);

    // ── The master volume mode ──
    edit("masterVolumeMode", 7);
    save();
    check("QD31: master volume mode: the bar says so", barSays("master volume mode"));
    check("QD31: master volume mode: the draft stays", dirty() && edited() && b->masterVolumeMode() == master);
    revert();
    check("QD31: master volume mode: reverted", clean());

    // ── A later step fails: the earlier one is on the device, and is off the draft ──
    edit("masterVolumeMode", master ? 0 : 1);
    edit("outputConfigMode", 7);
    save();
    check("QD31: output config mode: the bar says so", barSays("output config mode"));
    check("QD31: output config mode: the draft stays", dirty() && edited() && b->outputConfigMode() == output);
    check("QD31: the step before it went out", b->masterVolumeMode() == (master ? 0 : 1));
    // Put right, Save goes through and the reason goes
    edit("outputConfigMode", output);
    check("QD31: an edit drops the reason", reason().isEmpty());
    edit("masterVolumeMode", master);
    save();
    check("QD31: a good save ends the draft", clean() && b->masterVolumeMode() == master && b->outputConfigMode() == output);

    // ── A refused mode doesn't count as saved with the preset ──
    b->markHardwareUnsaved();
    check("QD31: hardware edits are unsaved", b->hardwareUnsaved() && dirty());
    check("QD31: a refused mode says so", !b->setOutputConfigMode(7));
    check("QD31: and leaves them unsaved", b->hardwareUnsaved());

    // ── The output config save itself ──
    check("QD31: the demo's save fails on its flash", dspi_demo_output_save_status(b->core(), 4));
    save();
    check("QD31: output config save: the bar says flash", barSays("flash"));
    check("QD31: output config save: still dirty", dirty() && b->hardwareUnsaved());
    check("QD31: the demo's save stalls", dspi_demo_output_save_status(b->core(), 0xFF));
    save();
    check("QD31: output config save: the bar says no response", barSays("did not respond"));
    check("QD31: output config save: still dirty after a stall", dirty() && b->hardwareUnsaved());
    check("QD31: the demo's save goes through again", dspi_demo_output_save_status(b->core(), 0));
    save();
    check("QD31: output config save: through Save, it is saved", clean() && !b->hardwareUnsaved());
    check("QD31: output config mode untouched", b->outputConfigMode() == output);

    // ── The DAC mute: the device has the last word, some time after the Save ──
    const QVariantMap before = b->hardware();
    const auto hw = [b](const char *key) { return b->hardware().value(QString::fromLatin1(key)); };
    edit("dacEnabled", true);
    edit("dacPin", 10);
    save();
    check("QD31: DAC mute: not saved until read back", dirty() && edited());
    check("QD31: DAC mute: the read-back ends the draft", waitFor(1000, clean));
    check("QD31: DAC mute: on the device", hw("dacMuteEnabled").toBool() && hw("dacMutePin").toInt() == 10);
    const bool low = hw("dacMuteActiveLow").toBool();
    const int release = hw("dacMuteReleaseMs").toInt();
    edit("dacHoldMs", 20);
    save();
    // What the device keeps isn't what was sent
    check("QD31: DAC mute: the device changes it", dspi_set_dac_mute(b->core(), true, low, 10, 7, uint16_t(release)));
    check("QD31: DAC mute: the bar says so", waitFor(1000, [&]() { return !reason().isEmpty(); }) && barSays("DAC mute"));
    check("QD31: DAC mute: the draft stays", dirty() && edited() && ctx->property("dacHoldMs").toInt() == 20);
    check("QD31: DAC mute: the bridge has what the device kept", hw("dacMuteHoldMs").toInt() == 7);
    save();
    check("QD31: DAC mute: Save again goes through", waitFor(1000, clean) && hw("dacMuteHoldMs").toInt() == 20);
    // An edit made while the read-back is on its way stays a draft
    edit("dacHoldMs", 30);
    save();
    edit("dacHoldMs", 40);
    spin(300);
    check("QD31: DAC mute: a newer edit stays", dirty() && edited() && ctx->property("dacHoldMs").toInt() == 40 && hw("dacMuteHoldMs").toInt() == 30);
    revert();
    check("QD31: DAC mute: and reverts to the device's", clean() && ctx->property("dacHoldMs").toInt() == 30);

    edit("dacEnabled", before.value("dacMuteEnabled"));
    edit("dacPin", before.value("dacMutePin"));
    edit("dacHoldMs", before.value("dacMuteHoldMs"));
    edit("dacActiveLow", before.value("dacMuteActiveLow"));
    edit("dacReleaseMs", before.value("dacMuteReleaseMs"));
    save();
    check("QD31: DAC mute: back as it was", waitFor(1000, clean) && hw("dacMuteEnabled") == before.value("dacMuteEnabled"));
    settings->setVisible(false);
}
