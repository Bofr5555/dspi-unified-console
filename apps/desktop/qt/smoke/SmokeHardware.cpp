// -- WP QP11 -- The hardware settings pages and the save bar: the pages list
// and search, Global Parameters' held drafts (At power-on, the latency and its
// held state, the device name), the live pin changes on Outputs, Inputs and
// I2S Configuration with the bar's Save and Revert, a refused save (QD31), the
// source names, and the control interfaces' own Apply. Every change made on
// the demo is put back. Screenshots of each page and state.
#include "SmokeDriver.h"

#include "Core.h"
#include "ChannelController.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "HardwareController.h"
#include "NamesController.h"
#include "OutputLatencyController.h"
#include "SettingsController.h"
#include "Shell.h"

#include "CoreApi.h"

#include <QQuickItem>
#include <QQuickWindow>

namespace {

QObject *find(QObject *root, const QString &name) {
    if (!root) return nullptr;
    if (root->objectName() == name) return root;
    if (auto *item = qobject_cast<QQuickItem *>(root)) {
        for (QQuickItem *child : item->childItems())
            if (QObject *o = find(child, name)) return o;
    }
    for (QObject *child : root->children())
        if (QObject *o = find(child, name)) return o;
    return nullptr;
}

struct Page {
    SmokeDriver &d;
    QQuickWindow *w;
    QObject *get(const QString &name) const { return w ? find(w->contentItem(), name) : nullptr; }
    bool shown(const QString &name) const {
        auto *item = qobject_cast<QQuickItem *>(get(name));
        return item && item->isVisible();
    }
    QString text(const QString &name) const {
        QObject *o = get(name);
        return o ? o->property("text").toString() : QString();
    }
    void click(const QString &name) const {
        if (QObject *o = get(name)) QMetaObject::invokeMethod(o, "click");
        d.spin(80);
    }
    // A ChoicePicker or PinPicker: pick the entry whose value is `value`, as a
    // person's choice does (its activated signal)
    bool choose(const QString &name, const QVariant &value) const {
        QObject *picker = get(name);
        if (!picker) return false;
        const QVariantList shown = picker->property("shown").toList();
        for (int i = 0; i < shown.size(); ++i) {
            if (shown.at(i).toMap().value(QStringLiteral("value")) == value) {
                QMetaObject::invokeMethod(picker, "activated", Q_ARG(int, i));
                d.spin(80);
                return true;
            }
        }
        return false;
    }
    // A NameField: type `text` into it
    void type(const QString &name, const QString &text) const {
        QObject *field = get(name + QStringLiteral(".field"));
        if (!field) return;
        field->setProperty("text", text);
        QMetaObject::invokeMethod(field, "textEdited");
        d.spin(50);
    }
    // The tab button at `index` of an AppTabBar
    void tab(const QString &name, int index) const {
        auto *bar = qobject_cast<QQuickItem *>(get(name));
        if (!bar) return;
        QQuickItem *button = nullptr;
        QMetaObject::invokeMethod(bar, "itemAt", Q_RETURN_ARG(QQuickItem *, button), Q_ARG(int, index));
        if (button) QMetaObject::invokeMethod(button, "click");
        d.spin(80);
    }
    bool barShown() const { return shown(QStringLiteral("SaveBar")); }
};

void open(SmokeDriver &d, SettingsController *s, const char *page) {
    s->open(QString::fromLatin1(page));
    d.spin(300);
    d.waitIdle();
    d.spin(100);
}

bool holds(const QVariantList &choices, int pin) {
    for (const QVariant &c : choices)
        if (c.toMap().value(QStringLiteral("pin")).toInt() == pin) return true;
    return false;
}

void hardwareSection(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    DeviceModel *device = d.app().device;
    auto *s = Registrar::instance<SettingsController>();
    auto *hw = Registrar::instance<HardwareController>();
    auto *names = Registrar::instance<NamesController>();
    auto *latency = Registrar::instance<OutputLatencyController>();
    d.check(QStringLiteral("qp11: the controllers exist"), s && hw && names && latency);
    if (!s || !hw || !names || !latency) return;
    d.waitIdle();
    d.check(QStringLiteral("qp11: the probe's answers arrive"),
            d.waitFor(3000, [&]() { return hw->has().value(QStringLiteral("inputSource")).toBool(); }));

    shell->trigger(QStringLiteral("edit.settings"));
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("edit.settings")));
    d.check(QStringLiteral("qp11: the Settings window opens"), w && d.waitFor(3000, [&]() { return w->isVisible(); }));
    if (!w) return;
    Page ui{ d, w };

    // ---- The pages in the sidebar, and the search ------------------------------------
    QStringList ids;
    for (const QVariant &row : s->sidebarRows(QString()))
        if (row.toMap().contains(QStringLiteral("id"))) ids << row.toMap().value(QStringLiteral("id")).toString();
    d.check(QStringLiteral("qp11: System lists Inputs, Outputs, I2S Configuration, Global Parameters after Overview"),
            ids.indexOf(QStringLiteral("overview")) >= 0 && ids.indexOf(QStringLiteral("inputs")) == ids.indexOf(QStringLiteral("overview")) + 1
                && ids.indexOf(QStringLiteral("outputs")) == ids.indexOf(QStringLiteral("inputs")) + 1
                && ids.indexOf(QStringLiteral("i2s")) == ids.indexOf(QStringLiteral("outputs")) + 1
                && ids.indexOf(QStringLiteral("global")) == ids.indexOf(QStringLiteral("i2s")) + 1);
    d.check(QStringLiteral("qp11: Control lists Control Interfaces on a device with them"),
            hw->has().value(QStringLiteral("interfaces")).toBool() == ids.contains(QStringLiteral("interfaces")));
    auto found = [&](const QString &query) {
        QStringList out;
        for (const QVariant &row : s->sidebarRows(query))
            if (row.toMap().contains(QStringLiteral("id"))) out << row.toMap().value(QStringLiteral("id")).toString();
        return out;
    };
    d.check(QStringLiteral("qp11: \"lat\" leaves Global Parameters"), found(QStringLiteral("lat")).contains(QStringLiteral("global")));
    d.check(QStringLiteral("qp11: \"pin\" leaves Outputs"), found(QStringLiteral("pin")).contains(QStringLiteral("outputs")));
    d.check(QStringLiteral("qp11: \"power-on\" leaves Global Parameters"),
            found(QStringLiteral("power-on")) == QStringList{ QStringLiteral("global") });
    d.check(QStringLiteral("qp11: \"MCK\" leaves I2S Configuration"), found(QStringLiteral("mck")).contains(QStringLiteral("i2s")));

    // ---- Global Parameters: held, then saved -------------------------------------------
    open(d, s, "global");
    d.check(QStringLiteral("qp11: Global Parameters shows"), ui.get(QStringLiteral("GlobalPage")) != nullptr);
    d.check(QStringLiteral("qp11: nothing held, no save bar"), !ui.barShown() && !hw->pending());
    auto *heading = qobject_cast<QQuickItem *>(ui.get(QStringLiteral("Global.powerOn")));
    auto below = [&](const char *name) {
        auto *item = qobject_cast<QQuickItem *>(ui.get(QString::fromLatin1(name)));
        return heading && item && item->isVisible() && item->y() > heading->y();
    };
    d.check(QStringLiteral("qp11: At power-on heads startup, master volume, hardware configuration and the external mute"),
            heading && heading->isVisible() && below("Global.startup") && below("Global.masterVolume")
                && below("Global.outputConfig")
                && (hw->has().value(QStringLiteral("dacMute")).toBool() ? below("Global.mute") : true));
    auto *latencyGroup = qobject_cast<QQuickItem *>(ui.get(QStringLiteral("Global.latency")));
    d.check(QStringLiteral("qp11: the latency card shows above the heading, with the device's options"),
            latency->supported() && latencyGroup && latencyGroup->isVisible() && latencyGroup->y() < heading->y()
                && latency->options().size() >= 2);
    d.check(QStringLiteral("qp11: the device name row shows"), names->deviceNameSupported() && ui.shown(QStringLiteral("Global.deviceName")));
    d.grab(w, QStringLiteral("settings-global.png"));

    const DspState *state = device->state();
    const int startMode = state ? state->preset_startup_mode : 0;
    const int volumeMode = state ? state->master_volume_mode : 0;
    const int settersBefore = device->settersRun();
    ui.choose(QStringLiteral("Global.startupMode"), startMode == 0 ? 1 : 0);
    d.check(QStringLiteral("qp11: a startup change is held: the bar shows"), hw->pending() && ui.barShown());
    d.check(QStringLiteral("qp11: and nothing is sent"), device->settersRun() == settersBefore
                                                             && device->state()->preset_startup_mode == startMode);
    d.check(QStringLiteral("qp11: the picker shows the held choice"),
            hw->draft().value(QStringLiteral("startupMode")).toInt() == (startMode == 0 ? 1 : 0));
    ui.click(QStringLiteral("SaveBar.revert"));
    d.check(QStringLiteral("qp11: Revert drops it and the bar goes"),
            !hw->pending() && !ui.barShown() && hw->draft().value(QStringLiteral("startupMode")).toInt() == startMode);

    ui.choose(QStringLiteral("Global.masterVolumeMode"), volumeMode == 0 ? 1 : 0);
    ui.click(QStringLiteral("SaveBar.save"));
    d.check(QStringLiteral("qp11: Save writes the master volume mode"),
            d.waitFor(3000, [&]() { return !hw->saving() && device->state()->master_volume_mode == (volumeMode == 0 ? 1 : 0); }));
    d.check(QStringLiteral("qp11: and the bar goes"), d.waitFor(1000, [&]() { return !hw->pending(); }) && hw->saveFailure().isEmpty());
    ui.choose(QStringLiteral("Global.masterVolumeMode"), volumeMode);
    hw->save();
    d.waitFor(3000, [&]() { return !hw->saving() && !hw->pending(); });
    d.check(QStringLiteral("qp11: the master volume mode is back"), device->state()->master_volume_mode == volumeMode);

    // ---- The latency's held state (behaviour.md §16) -------------------------------------
    const int latencyBefore = latency->requestedId();
    int other = -1;
    for (const QVariant &o : latency->options()) {
        const int id = o.toMap().value(QStringLiteral("id")).toInt();
        if (id != latencyBefore && o.toMap().value(QStringLiteral("available")).toBool()) {
            other = id;
            break;
        }
    }
    d.check(QStringLiteral("qp11: the demo offers another latency"), other >= 0);
    // The demo never applies the next request
    d.app().core->worker()->post([](FfiCore *c) { dspi_demo_latency_timing(c, 2, 0, 0); });
    ui.choose(QStringLiteral("Global.latencyTarget"), other);
    d.check(QStringLiteral("qp11: a latency choice is held"), hw->pending() && latency->selectedId() == other
                                                                  && latency->requestedId() == latencyBefore);
    hw->save();
    d.check(QStringLiteral("qp11: a latency the device has not applied keeps the bar with the held line"),
            d.waitFor(3000, [&]() { return !hw->saving() && latency->held(); }) && ui.barShown()
                && ui.shown(QStringLiteral("SaveBar.held")) && !hw->heldNote().isEmpty());
    d.check(QStringLiteral("qp11: the picker keeps the requested value"), latency->selectedId() == other);
    d.grab(w, QStringLiteral("settings-latency-held.png"));
    ui.click(QStringLiteral("SaveBar.revert"));
    d.check(QStringLiteral("qp11: Revert clears the held state"), !latency->held() && hw->heldNote().isEmpty());
    // At once from now on: a choice saves and reads applied
    d.app().core->worker()->post([](FfiCore *c) { dspi_demo_latency_timing(c, 0, 0, 0); });
    latency->choose(latencyBefore == other ? other : latencyBefore);
    latency->refresh();
    d.waitFor(2000, [&]() { return !device->busy(); });
    if (latency->requestedId() != latencyBefore) {
        latency->choose(latencyBefore);
        hw->save();
        d.waitFor(3000, [&]() { return !hw->saving() && !latency->held(); });
    }
    d.check(QStringLiteral("qp11: an applied latency leaves no held line, the latency is back"),
            d.waitFor(3000, [&]() { return !latency->held() && latency->requestedId() == latencyBefore; }));

    // ---- The device name (held, then saved) --------------------------------------------
    const QString nameBefore = device->deviceName();
    ui.type(QStringLiteral("Global.name"), QStringLiteral("Smoke Room\x01"));
    d.check(QStringLiteral("qp11: a name keeps printable ASCII only"), names->deviceName() == QLatin1String("Smoke Room"));
    d.check(QStringLiteral("qp11: a typed name is held"), names->deviceNameEdited() && hw->pending()
                                                            && device->deviceName() == nameBefore);
    d.grab(w, QStringLiteral("settings-names-global.png"));
    hw->save();
    d.check(QStringLiteral("qp11: Save names the device"),
            d.waitFor(3000, [&]() { return device->deviceName() == QLatin1String("Smoke Room"); }));
    d.check(QStringLiteral("qp11: and the name is no longer held"), d.waitFor(2000, [&]() { return !names->deviceNameEdited() && !hw->pending(); }));
    names->setDeviceName(nameBefore);
    hw->save();
    d.waitFor(3000, [&]() { return device->deviceName() == nameBefore && !hw->pending(); });
    d.check(QStringLiteral("qp11: the device name is back"), device->deviceName() == nameBefore);

    // ---- Outputs: a live pin change, Save refused (QD31), Revert ---------------------------
    if (device->state()->output_config_mode != 0) {
        hw->setDraft(QStringLiteral("outputConfigMode"), 0);
        hw->save();
        d.waitFor(3000, [&]() { return !hw->saving(); });
    }
    // -- WP QPI2 -- a limiter switched on the output card is output
    // configuration too: the bar offers to keep it (the Mac's beginOutputEdit)
    if (auto *cc = Registrar::instance<ChannelController>(); cc && cc->limiterSupported() && !hw->pending()) {
        const bool on = device->state()->limiter_enabled[0];
        cc->setLimiterEnabled(0, !on);
        d.check(QStringLiteral("qpi2: a limiter switched on the output card brings up the save bar"),
                d.waitFor(3000, [&]() { return device->state()->limiter_enabled[0] == !on && hw->pending(); }));
        cc->setLimiterEnabled(0, on);
        d.waitFor(3000, [&]() { return device->state()->limiter_enabled[0] == on; });
        hw->revert();
        d.check(QStringLiteral("qpi2: and Revert lets it go"), d.waitFor(3000, [&]() { return !hw->pending(); }));
    }
    open(d, s, "outputs");
    d.check(QStringLiteral("qp11: Outputs lists the slots and the sub"),
            ui.get(QStringLiteral("Outputs.row.0")) && ui.get(QStringLiteral("Outputs.row.4")));
    d.grab(w, QStringLiteral("settings-outputs.png"));
    const int pin0 = device->state()->output_pins[0];
    const QVariantList choices = hw->pinChoices(HardwareController::OutputPin, 0, pin0);
    d.check(QStringLiteral("qp11: a pin picker keeps its pin and leaves out the held ones"),
            holds(choices, pin0) && !holds(choices, device->state()->output_pins[1])
                && !holds(choices, device->state()->i2s_bck_pin));
    // The bus of slot 2 is the live change here
    const int type1 = device->state()->output_types[1];
    ui.choose(QStringLiteral("Outputs.type.1"), type1 == 0 ? 1 : 0);
    d.check(QStringLiteral("qp11: a bus change applies at once"),
            d.waitFor(3000, [&]() { return device->state()->output_types[1] == (type1 == 0 ? 1 : 0); }));
    d.check(QStringLiteral("qp11: the status line says so"),
            d.waitFor(1000, [&]() { return hw->statusText().startsWith(QLatin1String("Output 2 set to")) && !hw->statusError(); })
                && ui.shown(QStringLiteral("Outputs.status")));
    // A refused one says why
    // -- WP CG-SWS-b -- the demo answers pin requests now, and moving to the pin an
    // output already has is a no-op: refuse a move onto Output 2's pin (the core's
    // pin map refuses it before sending, PIN_IN_USE), then move to a free one
    const int pin1 = device->state()->output_pins[1];
    hw->setOutputPin(0, pin1);
    d.check(QStringLiteral("qp11: a refused pin change says so"),
            d.waitFor(3000, [&]() { return hw->statusError() && hw->statusText().contains(QString::number(pin1)); })
                && device->state()->output_pins[0] == pin0);
    int freePin = -1;
    for (const QVariant &c : choices) {
        const int p = c.toMap().value(QStringLiteral("pin")).toInt();
        if (p != pin0 && freePin < 0) freePin = p;
    }
    if (freePin >= 0) {
        hw->setOutputPin(0, freePin);
        d.check(QStringLiteral("qp11: a pin change to a free pin applies"),
                d.waitFor(3000, [&]() { return device->state()->output_pins[0] == freePin && !hw->statusError(); }));
        hw->setOutputPin(0, pin0);
        d.waitFor(3000, [&]() { return device->state()->output_pins[0] == pin0; });
    }
    d.check(QStringLiteral("qp11: and the bar offers to keep it"), hw->pending() && ui.barShown());
    d.app().core->worker()->post([](FfiCore *c) { dspi_demo_output_save_status(c, 1); });
    hw->save();
    d.check(QStringLiteral("qp11: a refused save keeps the bar with the orange line"),
            d.waitFor(3000, [&]() { return !hw->saving(); }) && hw->pending() && ui.shown(QStringLiteral("SaveBar.failure"))
                && hw->saveFailure().contains(QLatin1String("output configuration")));
    d.grab(w, QStringLiteral("settings-save-refused.png"));
    d.app().core->worker()->post([](FfiCore *c) { dspi_demo_output_save_status(c, 0); });
    ui.click(QStringLiteral("SaveBar.revert"));
    d.check(QStringLiteral("qp11: Revert puts the bus back"),
            d.waitFor(3000, [&]() { return device->state()->output_types[1] == type1; }) && !hw->pending());
    d.check(QStringLiteral("qp11: and the orange line goes"), hw->saveFailure().isEmpty() && !ui.barShown());

    // ---- I2S Configuration: the master clock --------------------------------------------
    open(d, s, "i2s");
    d.check(QStringLiteral("qp11: I2S Configuration shows"), ui.get(QStringLiteral("I2sPage")) != nullptr);
    d.check(QStringLiteral("qp11: the MCK pins are the chip's clock outputs"), [&]() {
        for (const QVariant &c : hw->mckPinChoices()) {
            const int p = c.toMap().value(QStringLiteral("pin")).toInt();
            if (p != 13 && p != 15 && p != 21) return false;
        }
        return !hw->mckPinChoices().isEmpty();
    }());
    d.grab(w, QStringLiteral("settings-i2s.png"));
    const bool mck = device->state()->mck_enabled;
    if (QObject *sw = ui.get(QStringLiteral("I2s.mck.switch"))) QMetaObject::invokeMethod(sw, "click");
    d.check(QStringLiteral("qp11: the MCK switch applies at once"), d.waitFor(3000, [&]() { return device->state()->mck_enabled != mck; }));
    hw->save();
    d.check(QStringLiteral("qp11: Save keeps the output configuration"),
            d.waitFor(3000, [&]() { return !hw->saving() && !hw->pending(); }) && hw->saveFailure().isEmpty());
    hw->setMckEnabled(mck);
    d.waitFor(3000, [&]() { return device->state()->mck_enabled == mck; });
    hw->save();
    d.waitFor(3000, [&]() { return !hw->saving() && !hw->pending(); });
    d.check(QStringLiteral("qp11: the master clock is back"), device->state()->mck_enabled == mck);

    // ---- Inputs: the S/PDIF count, the source names -----------------------------------------
    open(d, s, "inputs");
    d.check(QStringLiteral("qp11: Inputs shows S/PDIF and I2S input"),
            ui.shown(QStringLiteral("Inputs.spdif")) && ui.shown(QStringLiteral("Inputs.i2s")) == hw->has().value(QStringLiteral("i2sInput")).toBool());
    d.grab(w, QStringLiteral("settings-inputs.png"));
    if (hw->has().value(QStringLiteral("multiSpdif")).toBool()) {
        const int count = hw->hw().value(QStringLiteral("spdifEnabledCount")).toInt();
        const int want = count == 1 ? 2 : 1;
        ui.tab(QStringLiteral("Inputs.spdifCount"), want - 1);
        d.check(QStringLiteral("qp11: the S/PDIF count applies at once"),
                d.waitFor(3000, [&]() { return hw->hw().value(QStringLiteral("spdifEnabledCount")).toInt() == want; }));
        d.check(QStringLiteral("qp11: a row per input"), ui.get(QStringLiteral("Inputs.spdifPin.%1").arg(want - 1)) != nullptr || want == 1);
        ui.click(QStringLiteral("SaveBar.revert"));
        d.check(QStringLiteral("qp11: Revert puts the count back"),
                d.waitFor(3000, [&]() { return hw->hw().value(QStringLiteral("spdifEnabledCount")).toInt() == count; }) && !hw->pending());
    }
    d.check(QStringLiteral("qp11: the source names show, USB first"),
            names->sourceNamesSupported() && !names->sources().isEmpty()
                && names->sources().first().toMap().value(QStringLiteral("title")).toString() == QLatin1String("USB"));
    const QString usbBefore = names->sources().first().toMap().value(QStringLiteral("stored")).toString();
    ui.type(QStringLiteral("Inputs.name.0"), QStringLiteral("Smoke PC"));
    d.check(QStringLiteral("qp11: a source name is held"), hw->pending() && names->sources().first().toMap().value(QStringLiteral("edited")).toBool());
    d.grab(w, QStringLiteral("settings-names-inputs.png"));
    hw->save();
    d.check(QStringLiteral("qp11: Save names the source"),
            d.waitFor(3000, [&]() { return names->sources().first().toMap().value(QStringLiteral("stored")).toString() == QLatin1String("Smoke PC"); }));
    names->setSourceName(0, usbBefore);
    hw->save();
    d.waitFor(3000, [&]() { return !hw->pending(); });
    d.check(QStringLiteral("qp11: the source name is back"),
            d.waitFor(2000, [&]() { return names->sources().first().toMap().value(QStringLiteral("stored")).toString() == usbBefore; }));

    // ---- Control Interfaces: a draft of its own, its own Apply --------------------------------
    if (hw->has().value(QStringLiteral("interfaces")).toBool()) {
        open(d, s, "interfaces");
        d.check(QStringLiteral("qp11: Control Interfaces shows"), ui.get(QStringLiteral("InterfacesPage")) != nullptr);
        d.grab(w, QStringLiteral("settings-control.png"));
        QObject *page = ui.get(QStringLiteral("InterfacesPage"));
        if (QObject *sw = ui.get(QStringLiteral("Interfaces.uartEnabled.switch"))) QMetaObject::invokeMethod(sw, "click");
        d.spin(100);
        d.check(QStringLiteral("qp11: an interface edit waits for Apply"),
                page && page->property("uartDirty").toBool() && ui.text(QStringLiteral("Interfaces.uartApply.text")) == QLatin1String("Unapplied changes")
                    && !hw->pending());
        ui.click(QStringLiteral("Interfaces.uartApply.revert"));
        d.check(QStringLiteral("qp11: its Revert drops it"), page && !page->property("uartDirty").toBool());
        // Apply what the device has: the outcome comes back from the device
        hw->applyI2c(hw->hw().value(QStringLiteral("i2c")).toMap());
        d.check(QStringLiteral("qp11: Apply reports the device's outcome"),
                d.waitFor(3000, [&]() { return !hw->i2cOutcome().value(QStringLiteral("text")).toString().isEmpty(); }));
    }

    w->close();
    d.spin(100);
}

const SmokeDriver::Section hardware("qp11-hardware", 45, &hardwareSection);

} // namespace
