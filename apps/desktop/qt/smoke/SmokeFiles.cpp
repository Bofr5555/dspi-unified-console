// -- WP QP15 -- The File menu's section: filters out and back in (a Console
// file with the channel picker, a REW list, files that hold nothing), a device
// configuration out and back in with its options and report, the two saves,
// and the unsaved-changes question before a device switch, quitting and
// closing the window. Every change is undone before the section ends.
#include "Qp18Notices.h"   // -- WP QP18 --
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "Core.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "DevicesController.h"
#include "FilesController.h"
#include "PresetModel.h"
#include "Registrar.h"
#include "Shell.h"
#include "UnsavedGuard.h"

#include "CoreApi.h"

#include <QDir>
#include <QFile>
#include <QFileDialog>
#include <QGuiApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSignalSpy>
#include <QVariantList>

namespace {

QObject *find(QQuickItem *host, const char *name) { return host ? host->findChild<QObject *>(QLatin1String(name)) : nullptr; }

bool opened(QObject *popup) { return popup && popup->property("opened").toBool(); }

bool writeText(const QString &path, const QByteArray &text) {
    QFile f(path);
    return f.open(QIODevice::WriteOnly | QIODevice::Truncate) && f.write(text) == text.size();
}

// The next alert: its title and text, then closes it; empty if none came
struct Notice {
    QString title, text;
    bool error = false;
};
Notice takeNotice(SmokeDriver &d, QObject *notice, int ms = 8000) {
    Notice n;
    // -- WP QP18 -- or a success's notice in the banner stack
    if (!notice || !d.waitFor(ms, [&]() { return opened(notice) || qp18NoticeNew(); })) return n;
    if (!opened(notice)) {
        n.title = QStringLiteral("Success");
        n.text = qp18TakeNotice();
        return n;
    }
    n.title = notice->property("title").toString();
    n.text = notice->property("text").toString();
    n.error = notice->property("error").toBool();
    QMetaObject::invokeMethod(notice, "close");
    d.waitFor(2000, [&]() { return !opened(notice); });
    return n;
}

void press(QObject *button) {
    if (button) QMetaObject::invokeMethod(button, "clicked");
}

// Answers the open file dialog with `path`
bool answerDialog(SmokeDriver &d, const QString &path) {
    if (!d.waitFor(3000, []() { return FilesController::openDialog() && FilesController::openDialog()->isVisible(); }))
        return false;
    QFileDialog *dialog = FilesController::openDialog();
    dialog->selectFile(path);
    static_cast<QDialog *>(dialog)->accept();   // QFileDialog::accept, through the public base
    return true;
}

bool sameBand(const FilterParams &a, const FilterParams &b) {
    return a.filter_type == b.filter_type && a.freq == b.freq && a.q == b.q && a.gain == b.gain;
}

} // namespace

static void filesSection(SmokeDriver &d) {
    qp18NoticeMark();   // -- WP QP18 --
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    FilesController *files = Registrar::instance<FilesController>();
    UnsavedGuard *guard = Registrar::instance<UnsavedGuard>();
    ChannelController *channels = Registrar::instance<ChannelController>();
    DevicesController *devices = Registrar::instance<DevicesController>();
    d.check(QStringLiteral("qp15: the file controllers are made"), files && guard && channels && devices);
    if (!files || !guard || !channels || !devices) return;
    d.waitIdle();
    QQuickItem *host = files->host();
    d.check(QStringLiteral("qp15: the dialogs' host is in the main window"),
            host && host->window() == d.mainWindow());
    QObject *notice = find(host, "FilesHost.notice");
    QObject *picker = find(host, "ImportFilters");
    QObject *options = find(host, "ImportConfig");
    d.check(QStringLiteral("qp15: the alert, the picker and the options exist"), notice && picker && options);
    const QString dir = d.outDir() + QStringLiteral("/qp15");
    QDir().mkpath(dir);
    const QString filtersFile = dir + QStringLiteral("/filters.txt");
    QFile::remove(filtersFile);

    // ---- Export Filters through the menu row and its dialog ----
    const DspState start = *device->state();
    shell->trigger(QStringLiteral("file.exportFilters"));
    QFileDialog *dialog = FilesController::openDialog();
    d.check(QStringLiteral("qp15: Export Filters opens a save dialog"),
            d.waitFor(3000, [&]() { return (dialog = FilesController::openDialog()) && dialog->isVisible(); }) &&
                dialog->acceptMode() == QFileDialog::AcceptSave);
    d.check(QStringLiteral("qp15: it suggests DSPi Filters.txt"),
            dialog && !dialog->selectedFiles().isEmpty() && dialog->selectedFiles().first().endsWith(QLatin1String("DSPi Filters.txt")));
    d.check(QStringLiteral("qp15: the dialog takes a path"), answerDialog(d, filtersFile));
    Notice n = takeNotice(d, notice);
    d.check(QStringLiteral("qp15: the export says so"), n.title == QLatin1String("Success") && n.text == QLatin1String("Filters exported successfully"));
    QFile exported(filtersFile);
    const QByteArray filtersText = exported.open(QIODevice::ReadOnly) ? exported.readAll() : QByteArray();
    d.check(QStringLiteral("qp15: the file is a filter file with every input and output"),
            filtersText.startsWith("# DSPi") && filtersText.contains("[Input 0:") && filtersText.contains("[Output 0:"));

    // ---- Import it back: the Console file's picker ----
    const FilterParams band0 = start.filters[0][0];
    FilterParams changed = band0;
    changed.filter_type = FILTER_PEAKING;
    changed.freq = 2345.0f;
    changed.gain = 4.5f;
    changed.q = 2.0f;
    device->write(QStringLiteral("qp15 band"), [changed](FfiCore *c) { return dspi_set_filter(c, 0, 0, changed); });
    d.waitIdle();
    d.check(QStringLiteral("qp15: a band is changed"), sameBand(device->state()->filters[0][0], changed));
    files->importFiltersFrom(filtersFile);
    d.check(QStringLiteral("qp15: Import Filters shows the channel picker"), d.waitFor(5000, [&]() { return opened(picker); }));
    const QVariantList choices = picker->property("channels").toList();
    d.check(QStringLiteral("qp15: a Console file offers every channel it holds"),
            choices.size() == device->numInputs() + device->numOutputs());
    bool allTicked = !choices.isEmpty();
    for (const QVariant &c : choices) allTicked = allTicked && c.toMap().value(QStringLiteral("checked")).toBool();
    d.check(QStringLiteral("qp15: all ticked"), allTicked);
    d.check(QStringLiteral("qp15: the Console file's prompt"),
            picker->property("prompt").toString().startsWith(QLatin1String("This file contains filter settings for multiple channels")));
    d.check(QStringLiteral("qp15: a check box per channel"),
            d.item(QStringLiteral("ImportFilters.channel.0"), picker->property("contentItem").value<QQuickItem *>()) != nullptr);
    d.spin(200);
    d.check(QStringLiteral("qp15: screenshot of the filter picker"), d.grab(d.mainWindow(), QStringLiteral("window-files-import-filters.png")));
    for (const QVariant &c : choices) {
        const int wire = c.toMap().value(QStringLiteral("wire")).toInt();
        if (wire != 0) QMetaObject::invokeMethod(picker, "setTicked", Q_ARG(QVariant, wire), Q_ARG(QVariant, false));
    }
    press(find(host, "ImportFilters.import"));
    n = takeNotice(d, notice);
    d.check(QStringLiteral("qp15: the import reports one channel"),
            n.title == QLatin1String("Success") && n.text.startsWith(QLatin1String("Filters imported to 1 channel")));
    d.waitIdle();
    d.check(QStringLiteral("qp15: the band is back"), sameBand(device->state()->filters[0][0], band0));

    // Cancel sends nothing
    device->write(QStringLiteral("qp15 band"), [changed](FfiCore *c) { return dspi_set_filter(c, 0, 0, changed); });
    d.waitIdle();
    files->importFiltersFrom(filtersFile);
    d.waitFor(5000, [&]() { return opened(picker); });
    press(find(host, "ImportFilters.cancel"));
    d.waitFor(2000, [&]() { return !opened(picker); });
    d.spin(300);
    d.waitIdle();
    d.check(QStringLiteral("qp15: Cancel imports nothing"), sameBand(device->state()->filters[0][0], changed) && !opened(notice));

    // ---- A REW list: inputs ticked, outputs not; the preamp to the inputs ----
    const QString rewFile = dir + QStringLiteral("/rew.txt");
    writeText(rewFile, "Filter Settings file\n\nPreamp: -6.5 dB\n"
                       "Filter 1: ON PK Fc 1000 Hz Gain -3.0 dB Q 1.41\n"
                       "Filter 2: ON LSC Fc 105 Hz Gain 2.0 dB Q 0.71\n");
    files->importFiltersFrom(rewFile);
    d.check(QStringLiteral("qp15: a REW list shows the picker"), d.waitFor(5000, [&]() { return opened(picker); }));
    d.check(QStringLiteral("qp15: the REW prompt counts the filters and the preamp"),
            picker->property("prompt").toString() ==
                QLatin1String("Found 2 filter(s) and a -6.5 dB preamp. Select which channel(s) to apply them to:"));
    bool inputsTicked = true, outputsUnticked = true;
    int inputsOffered = 0;
    for (const QVariant &v : picker->property("channels").toList()) {
        const QVariantMap c = v.toMap();
        const bool input = c.value(QStringLiteral("wire")).toInt() < device->numInputs();
        if (input) {
            ++inputsOffered;
            inputsTicked = inputsTicked && c.value(QStringLiteral("checked")).toBool();
        } else {
            outputsUnticked = outputsUnticked && !c.value(QStringLiteral("checked")).toBool();
        }
    }
    d.check(QStringLiteral("qp15: the inputs are offered ticked"), inputsOffered == device->numInputs() && inputsTicked);
    d.check(QStringLiteral("qp15: the outputs are offered unticked"), outputsUnticked);
    press(find(host, "ImportFilters.import"));
    n = takeNotice(d, notice);
    d.check(QStringLiteral("qp15: the REW import reports the inputs"),
            n.title == QLatin1String("Success") && n.text.startsWith(QStringLiteral("Filters imported to %1 channel").arg(device->numInputs())));
    d.waitIdle();
    const FilterParams rew = device->state()->filters[1][0];
    d.check(QStringLiteral("qp15: the list's bands are on the inputs"),
            rew.filter_type == FILTER_PEAKING && rew.freq == 1000.0f && rew.gain == -3.0f &&
                device->state()->filters[0][1].filter_type == FILTER_LOWSHELF);
    d.check(QStringLiteral("qp15: and the preamp"), device->state()->input_preamp_db[0] == -6.5f);

    // Files that hold nothing
    const QString emptyFile = dir + QStringLiteral("/nothing.txt");
    writeText(emptyFile, "just some text\n");
    files->importFiltersFrom(emptyFile);
    n = takeNotice(d, notice);
    d.check(QStringLiteral("qp15: a file without filters is refused"), n.error && n.text == QLatin1String("No valid filters found in file"));
    files->importFiltersFrom(dir + QStringLiteral("/missing.txt"));
    n = takeNotice(d, notice);
    d.check(QStringLiteral("qp15: a file that is not there is refused"), n.error && n.text.startsWith(QLatin1String("Failed to read file:")));
    d.check(QStringLiteral("qp15: no picker for them"), !opened(picker));

    // Everything back: the exported file on every channel
    files->importFiltersFrom(filtersFile);
    d.waitFor(5000, [&]() { return opened(picker); });
    press(find(host, "ImportFilters.import"));
    n = takeNotice(d, notice);
    d.waitIdle();
    const int inputs = device->numInputs();
    device->write(QStringLiteral("qp15 preamps"), [start, inputs](FfiCore *c) {
        bool ok = true;
        for (int i = 0; i < inputs; ++i) ok = dspi_set_input_preamp(c, uint8_t(i), start.input_preamp_db[i]) && ok;
        return ok;
    });
    d.waitIdle();
    bool restored = true;
    for (int ch = 0; ch < device->numInputs(); ++ch)
        for (int b = 0; b < BANDS_PER_CHANNEL; ++b) restored = restored && sameBand(device->state()->filters[ch][b], start.filters[ch][b]);
    d.check(QStringLiteral("qp15: importing the exported file restores every input"), restored);

    // ---- Export Device Configuration ----
    const QString configFile = dir + QStringLiteral("/Bench.dspipreset");
    QFile::remove(configFile);
    shell->trigger(QStringLiteral("file.exportConfiguration"));
    d.check(QStringLiteral("qp15: Export Device Configuration opens a save dialog"), answerDialog(d, configFile));
    n = takeNotice(d, notice);
    d.check(QStringLiteral("qp15: the export's summary"),
            n.title == QLatin1String("Success") && n.text.startsWith(QLatin1String("Configuration exported.")) &&
                n.text.contains(QLatin1String("active EQ bands")) && n.text.contains(QLatin1String("crosspoints.")));
    QFile config(configFile);
    const QJsonObject doc = config.open(QIODevice::ReadOnly) ? QJsonDocument::fromJson(config.readAll()).object() : QJsonObject();
    d.check(QStringLiteral("qp15: the document is the core's, named after the file"),
            doc.value(QLatin1String("schemaVersion")).toInt() == 1 &&
                doc.value(QLatin1String("meta")).toObject().value(QLatin1String("name")).toString() == QLatin1String("Bench"));

    // ---- Import it back: the options, the report, the links ----
    const float gain = device->state()->output_gain_db[0];
    const bool pairLinked = channels->isLinked(0);
    device->write(QStringLiteral("qp15 gain"), [gain](FfiCore *c) { return dspi_set_output_gain(c, 0, gain - 2.0f); });
    if (pairLinked) channels->unlinkPair(0);
    else channels->linkPair(0, -1);
    d.waitIdle();
    shell->trigger(QStringLiteral("file.importConfiguration"));
    d.check(QStringLiteral("qp15: Import Device Configuration opens a file dialog"), answerDialog(d, configFile));
    d.check(QStringLiteral("qp15: the options are asked"), d.waitFor(5000, [&]() { return opened(options); }));
    const QString prompt = options->property("prompt").toString();
    // -- WP P1QT1 -- the blocks are chosen below the prompt now, one box each
    // (a Repeater's: found in the dialog's items)
    d.check(QStringLiteral("qp15: the prompt says where the file came from"),
            prompt.startsWith(QStringLiteral("Saved from %1, firmware ").arg(device->platformName())));
    QQuickItem *optionItems = options->property("contentItem").value<QQuickItem *>();
    QObject *volumes = optionItems ? d.item(QStringLiteral("ImportConfig.volumes"), optionItems) : nullptr;
    QObject *hardware = optionItems ? d.item(QStringLiteral("ImportConfig.hardware"), optionItems) : nullptr;
    d.check(QStringLiteral("qp15: volumes and hardware start unticked"),
            volumes && hardware && !volumes->property("checked").toBool() && !hardware->property("checked").toBool());
    d.spin(200);
    d.check(QStringLiteral("qp15: screenshot of the import options"), d.grab(d.mainWindow(), QStringLiteral("window-files-import-config.png")));
    press(find(host, "ImportConfig.import"));
    n = takeNotice(d, notice, 15000);
    d.check(QStringLiteral("qp15: the import's report"),
            !n.error && n.text.startsWith(QLatin1String("Applied ")) &&
                n.text.endsWith(QLatin1String("Save them to a preset slot to keep them.")));
    d.waitIdle();
    d.check(QStringLiteral("qp15: the configuration is applied"), device->state()->output_gain_db[0] == gain);
    d.check(QStringLiteral("qp15: the input links come back with it"), channels->isLinked(0) == pairLinked);
    d.check(QStringLiteral("qp15: the progress sheet is gone"), !files->importing());

    // Not a configuration
    const QString badFile = dir + QStringLiteral("/bad.dspipreset");
    writeText(badFile, "{\"hello\": 1}");
    files->importConfigurationFrom(badFile);
    n = takeNotice(d, notice);
    d.check(QStringLiteral("qp15: a file that is no configuration is refused"), n.error && !n.text.isEmpty() && !opened(options));

    // ---- The saves ----
    shell->trigger(QStringLiteral("file.saveMasterVolume"));
    n = takeNotice(d, notice);
    d.check(QStringLiteral("qp15: Save Master Volume says what it stored"),
            !n.error && n.text.startsWith(QLatin1String("Master volume saved (")) && n.text.endsWith(QLatin1String("It will be applied on next boot.")));
    shell->trigger(QStringLiteral("file.saveOutputConfiguration"));
    n = takeNotice(d, notice);
    d.check(QStringLiteral("qp15: Save Output Configuration says so"),
            !n.error && n.text == QLatin1String("Output configuration saved. It will be applied on next boot."));
    d.app().core->worker()->post([](FfiCore *c) { dspi_demo_output_save_status(c, 1); });
    shell->trigger(QStringLiteral("file.saveOutputConfiguration"));
    n = takeNotice(d, notice);
    d.check(QStringLiteral("qp15: a refused save says so"),
            n.error && n.text == QLatin1String("Failed to save output configuration. The device did not acknowledge the request."));
    d.app().core->worker()->post([](FfiCore *c) { dspi_demo_output_save_status(c, 0); });
    d.waitIdle();

    // ---- -- WP QPI2 -- the Device menu's flash rows ----
    QObject *deviceQuestion = find(host, "FilesHost.deviceQuestion");
    shell->trigger(QStringLiteral("device.commit"));
    d.check(QStringLiteral("qpi2: Commit Parameters asks first, naming the slot"),
            deviceQuestion && d.waitFor(3000, [&]() { return opened(deviceQuestion); })
                && deviceQuestion->property("title").toString() == QLatin1String("Save Preset")
                && deviceQuestion->property("text").toString()
                       == QStringLiteral("Save current parameters to preset slot %1?").arg(device->presets()->activeSlot() + 1));
    if (deviceQuestion) QMetaObject::invokeMethod(deviceQuestion, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    n = takeNotice(d, notice);
    d.check(QStringLiteral("qpi2: and saves the preset"), !n.error && n.text == QLatin1String("Preset saved successfully"));
    const float kept = device->state()->output_gain_db[0];
    device->write(QStringLiteral("qpi2 gain"), [kept](FfiCore *c) { return dspi_set_output_gain(c, 0, kept - 2.0f); });
    d.waitIdle();
    shell->trigger(QStringLiteral("device.revert"));
    d.check(QStringLiteral("qpi2: Revert to Saved asks first"),
            deviceQuestion && d.waitFor(3000, [&]() { return opened(deviceQuestion); })
                && deviceQuestion->property("title").toString() == QLatin1String("Revert to Saved"));
    if (deviceQuestion) QMetaObject::invokeMethod(deviceQuestion, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    n = takeNotice(d, notice);
    d.check(QStringLiteral("qpi2: and loads the saved preset back"),
            !n.error && n.text == QLatin1String("Parameters reverted successfully")
                && d.waitFor(3000, [&]() { return device->state()->output_gain_db[0] == kept; }));
    shell->trigger(QStringLiteral("device.factoryReset"));
    d.check(QStringLiteral("qpi2: Factory Reset asks first, as a destructive question"),
            deviceQuestion && d.waitFor(3000, [&]() { return opened(deviceQuestion); })
                && deviceQuestion->property("title").toString() == QLatin1String("Factory Reset")
                && deviceQuestion->property("destructive").toBool());
    d.check(QStringLiteral("qpi2: screenshot of the factory reset question"), d.grab(d.mainWindow(), QStringLiteral("window-device-factory-reset.png")));
    const int writesBefore = device->settersRun();
    if (deviceQuestion) QMetaObject::invokeMethod(deviceQuestion, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.spin(300);
    d.check(QStringLiteral("qpi2: Cancel resets nothing"), !opened(notice) && device->settersRun() == writesBefore);

    // ---- The unsaved-changes question ----
    QObject *question = find(host, "FilesHost.unsavedQuestion");
    d.check(QStringLiteral("qp15: the question exists"), question != nullptr);
    if (!question) return;
    const QString demoSerial = device->serial();

    // An edit sent just before the question still counts (the settle)
    device->write(QStringLiteral("qp15 gain"), [gain](FfiCore *c) { return dspi_set_output_gain(c, 0, gain - 1.0f); });
    devices->chooseUsb(QStringLiteral("QP15-NONE"));
    d.check(QStringLiteral("qp15: picking another device asks first"), d.waitFor(3000, [&]() { return opened(question); }));
    d.check(QStringLiteral("qp15: an edit sent just before counts"), question->property("text").toString().contains(QStringLiteral("• ")));
    d.check(QStringLiteral("qp15: the guard is asking"), guard->asking());
    d.spin(200);
    d.check(QStringLiteral("qp15: screenshot of the device switch's question"), d.grab(d.mainWindow(), QStringLiteral("window-files-unsaved.png")));
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.waitFor(2000, [&]() { return !guard->asking(); });
    d.waitIdle();
    d.check(QStringLiteral("qp15: Cancel stays on the device"), device->connected() && device->serial() == demoSerial);

    // The same device again needs no question
    devices->chooseDemo();
    d.spin(300);
    d.check(QStringLiteral("qp15: choosing the open device does not ask"), !opened(question) && !guard->asking());
    d.waitIdle();
    // (chooseDemo reopened the demo: the edit is gone; make it again)
    device->write(QStringLiteral("qp15 gain"), [gain](FfiCore *c) { return dspi_set_output_gain(c, 0, gain - 1.0f); });
    d.waitIdle();

    // Quit asks, and Cancel keeps the app
    shell->trigger(QStringLiteral("file.quit"));
    d.check(QStringLiteral("qp15: Quit asks first"), d.waitFor(3000, [&]() { return opened(question); }));
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.waitFor(2000, [&]() { return !guard->asking(); });
    d.spin(200);
    d.check(QStringLiteral("qp15: Cancel keeps the app running"), d.mainWindow()->isVisible());

    // Closing the window asks too
    d.mainWindow()->close();
    d.check(QStringLiteral("qp15: closing the window asks first"), d.waitFor(3000, [&]() { return opened(question); }));
    d.check(QStringLiteral("qp15: the window stays while it asks"), d.mainWindow()->isVisible());
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.waitFor(2000, [&]() { return !guard->asking(); });
    d.spin(200);
    d.check(QStringLiteral("qp15: Cancel keeps the window"), d.mainWindow()->isVisible());

    // A quit from the tray while the window is hidden: the window comes back for the question
    d.mainWindow()->hide();
    d.spin(100);
    shell->trigger(QStringLiteral("file.quit"));
    d.check(QStringLiteral("qp15: a quit with the window hidden asks"), d.waitFor(3000, [&]() { return opened(question); }));
    d.check(QStringLiteral("qp15: and brings the window back for it"), d.mainWindow()->isVisible());
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    d.waitFor(2000, [&]() { return !guard->asking(); });

    // A close that only hides the window (the tray keeps the app) asks nothing
    QGuiApplication::setQuitOnLastWindowClosed(false);
    d.mainWindow()->close();
    d.spin(300);
    d.check(QStringLiteral("qp15: a close that only hides asks nothing"), !opened(question) && !guard->asking());
    d.mainWindow()->show();
    QGuiApplication::setQuitOnLastWindowClosed(true);
    d.spin(100);

    // Save stores the live state, then switches
    QSignalSpy failed(guard, &UnsavedGuard::saveFailed);
    devices->chooseUsb(QStringLiteral("QP15-NONE"));
    d.waitFor(3000, [&]() { return opened(question); });
    QMetaObject::invokeMethod(question, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    d.check(QStringLiteral("qp15: Save then switches"), d.waitFor(8000, [&]() { return device->serial() == QLatin1String("QP15-NONE"); }));
    d.check(QStringLiteral("qp15: the save went through"), failed.isEmpty() && !guard->asking());
    d.waitIdle();

    // Back to the demo, as it was
    devices->chooseDemo();
    d.check(QStringLiteral("qp15: the demo opens again"),
            d.waitFor(8000, [&]() { return device->connected() && device->demo(); }) && !opened(question));
    d.waitIdle();
    if (device->state()->output_gain_db[0] != gain) {
        device->write(QStringLiteral("qp15 gain"), [gain](FfiCore *c) { return dspi_set_output_gain(c, 0, gain); });
        const int slot = device->presets()->activeSlot();
        if (slot >= 0) device->write(QStringLiteral("preset save"), [slot](FfiCore *c) { return dspi_save_preset(c, uint8_t(slot)) == PRESET_OK; });
        d.waitIdle();
    }
    d.check(QStringLiteral("qp15: the demo is back as it was"), device->state()->output_gain_db[0] == gain);
}

static const SmokeDriver::Section files("qp15-files", 150, &filesSection);
