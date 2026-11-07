// WP QD18: the host's audio format and the fallback to the device's report, through
// a fake backend. The real tools are never run here: the Linux backend runs against
// scripts and a file tree, and the demo device never reaches the real backend.
#include "SmokeDriver.h"

#include "DSPiBridge.h"
#include "HostAudioFormat.h"

#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QMutex>
#include <QMutexLocker>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickItem>
#include <QQuickWindow>
#include <QThread>
#include <atomic>
#include <memory>

using HostAudio::Endpoint;

namespace {
QQuickItem *findItem(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = findItem(c, name)) return found;
    return nullptr;
}

struct FakeHost {
    QMutex lock;
    std::vector<Endpoint> endpoints;
    std::function<void()> changed;
    std::atomic<int> reads { 0 };
    std::atomic<bool> offUiThread { true };
    std::atomic<bool> watching { false };

    void set(std::vector<Endpoint> list) {
        std::function<void()> tell;
        {
            QMutexLocker hold(&lock);
            endpoints = std::move(list);
            tell = changed;
        }
        if (tell) tell();
    }
};

class FakeBackend : public HostAudio::Backend {
public:
    explicit FakeBackend(std::shared_ptr<FakeHost> host) : m_host(std::move(host)) {}
    std::vector<Endpoint> read() override {
        m_host->reads++;
        if (QThread::currentThread() == QCoreApplication::instance()->thread()) m_host->offUiThread = false;
        QMutexLocker hold(&m_host->lock);
        return m_host->endpoints;
    }
    void startWatching(std::function<void()> changed) override {
        QMutexLocker hold(&m_host->lock);
        m_host->changed = std::move(changed);
        m_host->watching = true;
    }
    void stopWatching() override {
        QMutexLocker hold(&m_host->lock);
        m_host->changed = nullptr;
        m_host->watching = false;
    }

private:
    std::shared_ptr<FakeHost> m_host;
};

Endpoint dspi(int channels, const QString &serial = QString()) {
    Endpoint e;
    e.text = QStringLiteral("Speakers (Weeb Labs DSPi)");
    e.serialText = serial;
    e.channels = channels;
    return e;
}

void writeFile(const QString &path, const QByteArray &data, bool executable = false) {
    QDir().mkpath(path.left(path.lastIndexOf(QLatin1Char('/'))));
    QFile f(path);
    if (f.open(QIODevice::WriteOnly | QIODevice::Truncate)) f.write(data);
    f.close();
    if (executable) f.setPermissions(f.permissions() | QFile::ExeOwner);
}

// Two nodes of one real shape, numbers once as a string, and what must not count
const char kDump[] = R"([
  {"id":1,"type":"PipeWire:Interface:Core","info":{"props":{"media.class":"Audio/Sink"}}},
  {"id":40,"type":"PipeWire:Interface:Node","info":{"props":{
     "media.class":"Audio/Source","device.api":"alsa","audio.channels":2,
     "node.name":"alsa_input.usb-Weeb_Labs_DSPi_E6614-00.mono"}}},
  {"id":41,"type":"PipeWire:Interface:Node","info":{"props":{
     "media.class":"Audio/Sink","factory.name":"support.null-audio-sink","audio.channels":2,
     "node.name":"DSPi virtual sink","node.description":"DSPi filter chain"}}},
  {"id":42,"type":"PipeWire:Interface:Node","info":{"props":{
     "media.class":"Audio/Sink","device.api":"alsa","audio.channels":2,
     "node.name":"alsa_output.pci-0000_00_1f.3.analog-stereo","node.description":"Built-in Audio"}}},
  {"id":43,"type":"PipeWire:Interface:Node","info":{"props":{
     "media.class":"Audio/Sink","api.alsa.path":"front:1","audio.channels":8,
     "node.name":"alsa_output.usb-Weeb_Labs_DSPi_E6614-00.analog-surround-71",
     "node.description":"Weeb Labs DSPi Surround 7.1"}}},
  {"id":44,"type":"PipeWire:Interface:Node","info":{"props":{
     "media.class":"Audio/Sink","device.api":"alsa","audio.channels":"6",
     "node.name":"alsa_output.usb-Weeb_Labs_DSPi_B0B0-00.analog-surround-51"}}},
  {"id":45,"type":"PipeWire:Interface:Node","info":{"props":{
     "media.class":"Audio/Sink","device.api":"alsa","audio.position":"FL,FR,RL,RR",
     "node.name":"alsa_output.usb-Weeb_Labs_DSPi_C0C0-00.analog-surround-40"}}},
  {"id":46,"type":"PipeWire:Interface:Node","info":{"params":{"Format":[{"channels":4}]},"props":{
     "media.class":"Audio/Sink","device.api":"alsa",
     "node.name":"alsa_output.usb-Weeb_Labs_DSPi_D0D0-00.pro-output-0"}}},
  {"id":47,"type":"PipeWire:Interface:Node","info":{"props":{
     "media.class":"Audio/Sink","device.api":"alsa","audio.channels":100000,
     "node.name":"alsa_output.usb-Weeb_Labs_DSPi_F0F0-00.junk"}}},
  null, 7, {"type":"PipeWire:Interface:Node"}, {"type":"PipeWire:Interface:Node","info":{"props":null}}
])";
}

void SmokeDriver::testHostAudio() {
    using namespace HostAudio;

    section("host audio format: choosing and parsing (QD18)");
    // ── Which endpoint ──
    check("QD18: nothing there, no answer", resolve({}, QStringLiteral("S1")) == 0);
    {
        Endpoint other;
        other.text = QStringLiteral("Built-in Audio");
        other.channels = 8;
        check("QD18: another device's output doesn't count", resolve({ other }, QString()) == 0);
    }
    check("QD18: one DSPi with no serial to match", resolve({ dspi(6) }, QStringLiteral("S1")) == 6);
    check("QD18: the name match ignores case", [] {
        Endpoint e = dspi(4);
        e.text = QStringLiteral("usb weeb labs dspi");
        return resolve({ e }, QString()) == 4;
    }());
    check("QD18: the serial picks the unit", resolve({ dspi(8, QStringLiteral("AAA")), dspi(4, QStringLiteral("bbb")) }, QStringLiteral("BBB")) == 4);
    check("QD18: two units disagree and no serial tells them apart", resolve({ dspi(8), dspi(4) }, QStringLiteral("S1")) == 0);
    check("QD18: two units agree", resolve({ dspi(8), dspi(8) }, QStringLiteral("S1")) == 8);
    check("QD18: an endpoint with no count says nothing", resolve({ dspi(0), dspi(6) }, QString()) == 6);

    // ── The count, per source (Swift's effectiveInputChannels) ──
    check("QD18: USB follows the host through an idle stream", effectiveInputCount(0, 2, 2, 8, 8) == 8);
    check("QD18: USB falls back to the device's report", effectiveInputCount(0, 2, 6, 0, 8) == 6);
    check("QD18: the report's 0 from old firmware is still a stereo pair", effectiveInputCount(0, 0, 0, 0, 8) == 2);
    check("QD18: S/PDIF ignores the host", effectiveInputCount(1, 8, 8, 8, 8) == 2 && effectiveInputCount(6, 8, 8, 8, 8) == 2);
    check("QD18: I2S takes its setting", effectiveInputCount(2, 4, 2, 8, 8) == 4);
    check("QD18: ADAT is eight", effectiveInputCount(3, 2, 2, 2, 8) == 8);
    check("QD18: held to what the device has", effectiveInputCount(0, 2, 2, 8, 2) == 2);

    check("QD18: a Link session has a hub behind it", phaseHasHub(QStringLiteral("bound")) && phaseHasHub(QStringLiteral("reconnecting"))
                                                       && !phaseHasHub(QStringLiteral("idle")) && !phaseHasHub(QStringLiteral("closed")));

    // ── WAVEFORMATEX, the part of the Windows path that runs anywhere ──
    const char format[] = { 1, 0, 8, 0 };
    check("QD18: nChannels of a device format", waveFormatChannels(format, 4) == 8);
    check("QD18: a short blob is no format", waveFormatChannels(format, 3) == 0 && waveFormatChannels(nullptr, 0) == 0);

#ifdef Q_OS_LINUX
    // ── pw-dump ──
    bool ok = false;
    const std::vector<Endpoint> sinks = parsePipeWireDump(QByteArray(kDump), &ok);
    check("QD18: a dump parses; only ALSA sinks count", ok && sinks.size() == 6);
    check("QD18: serial out of node.name, channels from audio.channels", resolve(sinks, QStringLiteral("E6614")) == 8);
    check("QD18: channels as a string", resolve(sinks, QStringLiteral("B0B0")) == 6);
    check("QD18: channels from audio.position", resolve(sinks, QStringLiteral("C0C0")) == 4);
    check("QD18: channels from the format", resolve(sinks, QStringLiteral("D0D0")) == 4);
    check("QD18: an absurd count is no count", sinks.back().channels == 0);
    parsePipeWireDump(QByteArrayLiteral("{\"not\":\"a dump\"}"), &ok);
    check("QD18: not an array", !ok);
    parsePipeWireDump(QByteArrayLiteral("[{\"tru"), &ok);
    check("QD18: cut short", !ok);
    parsePipeWireDump(QByteArray(), &ok);
    check("QD18: empty", !ok);

    // ── /proc/asound, as a file tree ──
    const QString tree = m_out + QStringLiteral("/hostaudio");
    QDir(tree).removeRecursively();
    const QString proc = tree + QStringLiteral("/asound"), sys = tree + QStringLiteral("/sound");
    writeFile(proc + QStringLiteral("/cards"),
              " 0 [PCH            ]: HDA-Intel - HDA Intel PCH\n"
              "                      HDA Intel PCH at 0xf7210000 irq 31\n"
              " 1 [DSPi           ]: USB-Audio - Weeb Labs DSPi\n"
              "                      Weeb Labs Weeb Labs DSPi at usb-0000:00:14.0-2, full speed\n"
              " 2 [DSPi_1         ]: USB-Audio - Weeb Labs DSPi\n"
              "                      Weeb Labs Weeb Labs DSPi at usb-0000:00:14.0-3, full speed\n"
              "not a card\n");
    writeFile(proc + QStringLiteral("/card0/pcm0p/sub0/hw_params"), "access: RW\nchannels: 2\n");
    writeFile(proc + QStringLiteral("/card1/usbid"), "2e8a:feaa\n");
    writeFile(proc + QStringLiteral("/card1/pcm0p/sub0/hw_params"), "closed\n");
    writeFile(proc + QStringLiteral("/card2/usbid"), "2e8a:feaa\n");
    writeFile(proc + QStringLiteral("/card2/pcm0c/sub0/hw_params"), "access: RW\nchannels: 1\n");
    writeFile(proc + QStringLiteral("/card2/pcm0p/sub0/hw_params"), "access: RW_INTERLEAVED\nformat: S24_3LE\nchannels: 6\nrate: 48000\n");
    QDir().mkpath(sys + QStringLiteral("/card1/device"));
    writeFile(sys + QStringLiteral("/card1/serial"), "SER1\n");
    QDir().mkpath(sys + QStringLiteral("/card2/device"));
    writeFile(sys + QStringLiteral("/card2/serial"), "SER2\n");
    const std::vector<Endpoint> cards = readAlsa(proc, sys);
    check("QD18: the USB cards only", cards.size() == 2);
    check("QD18: a closed stream has no count", cards.size() == 2 && cards[0].channels == 0 && cards[0].serialText == QLatin1String("SER1"));
    check("QD18: an open one has, playback side", cards.size() == 2 && cards[1].channels == 6);
    check("QD18: idle ALSA means the device's report", resolve({ cards[0] }, QStringLiteral("SER1")) == 0);
    check("QD18: no /proc/asound", readAlsa(tree + QStringLiteral("/none"), sys).empty());

    // ── The backend with its tool: scripts stand in for pw-dump ──
    LinuxPaths paths;
    paths.procAsound = proc;
    paths.sysClassSound = sys;
    paths.timeoutMs = 300;
    paths.pwDump = tree + QStringLiteral("/no-such-tool");
    check("QD18: no pw-dump, ALSA answers", resolve(makeLinuxBackend(paths)->read(), QStringLiteral("SER2")) == 6);

    const QString tool = tree + QStringLiteral("/pw-dump");
    paths.pwDump = tool;
    writeFile(tree + QStringLiteral("/dump.json"), QByteArray(kDump));
    writeFile(tool, "#!/bin/sh\nexec cat \"" + tree.toUtf8() + "/dump.json\"\n", true);
    check("QD18: pw-dump answers first", resolve(makeLinuxBackend(paths)->read(), QStringLiteral("E6614")) == 8);
    writeFile(tool, "#!/bin/sh\necho '[{\"type\":\"PipeWire:Interface:Node\"}]'\n", true);
    check("QD18: a dump with no DSPi, ALSA answers", resolve(makeLinuxBackend(paths)->read(), QStringLiteral("SER2")) == 6);
    writeFile(tool, "#!/bin/sh\necho no daemon >&2\nexit 1\n", true);
    check("QD18: pw-dump fails, ALSA answers", resolve(makeLinuxBackend(paths)->read(), QStringLiteral("SER2")) == 6);
    writeFile(tool, "#!/bin/sh\nexec sleep 60\n", true);
    check("QD18: pw-dump hangs, it's killed and ALSA answers", resolve(makeLinuxBackend(paths)->read(), QStringLiteral("SER2")) == 6);

    // The monitor: output means "read again", and stopping ends it
    const QString pidFile = tree + QStringLiteral("/monitor.pid");
    writeFile(tool, "#!/bin/sh\necho $$ > \"" + pidFile.toUtf8() + "\"\necho '[]'\nexec sleep 60\n", true);
    {
        std::atomic<int> told { 0 };
        std::unique_ptr<Backend> watcher = makeLinuxBackend(paths);
        watcher->startWatching([&told]() { told++; });
        check("QD18: the monitor's output asks for a read", waitFor(3000, [&]() { return told > 0; }));
        watcher->stopWatching();
        QFile pid(pidFile);
        const QString pidText = pid.open(QIODevice::ReadOnly) ? QString::fromUtf8(pid.readAll()).trimmed() : QString();
        check("QD18: stopping ends the monitor", !pidText.isEmpty() && !QFile::exists(QStringLiteral("/proc/") + pidText));
    }
    writeFile(tool, "#!/bin/sh\nexit 1\n", true);
    {
        std::atomic<int> told { 0 };
        std::unique_ptr<Backend> watcher = makeLinuxBackend(paths);
        watcher->startWatching([&told]() { told++; });
        check("QD18: a monitor that can't run says so once", waitFor(3000, [&]() { return told > 0; }));
        spin(200);
        check("QD18: and isn't started again", told == 1);
        watcher->stopWatching();
    }
    QDir(tree).removeRecursively();
#endif

    // ── The controller on the demo device, through the real QML ──
    section("host audio format: fallback, through the app (QD18)");
    auto *host = qobject_cast<HostAudioFormatController *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("hostAudio")).value<QObject *>());
    check("QD18: registered as hostAudio", host != nullptr);
    if (!host) return;
    auto *b = m_.bridge;
    check("QD18: the demo device is open", b->connected());
    auto *qroot = qobject_cast<QQuickWindow *>(root());
    QQuickItem *strips = qroot ? findItem(qroot->contentItem(), QStringLiteral("sidebarInputs")) : nullptr;
    check("QD18: the sidebar's input strips", strips != nullptr);
    QObject *leveller = root()->findChild<QObject *>(QStringLiteral("levellerWindow"));
    QObject *matrix = root()->findChild<QObject *>(QStringLiteral("matrixWindow"));
    check("QD18: the leveller and matrix windows", leveller && matrix);
    if (!strips || !leveller || !matrix) return;
    const auto stripCount = [strips]() { return strips->property("count").toInt(); };
    const int report = qMin(b->numInputChannels(), qMax(2, b->activeInputChannels()));
    check("QD18: the demo device reports fewer than it has, like an idle stream", report < b->numInputChannels());
    const int matrixFallback = matrix->property("inputCount").toInt();

    auto fake = std::make_shared<FakeHost>();
    fake->endpoints = { dspi(8) };
    int emitted = 0;
    const QMetaObject::Connection counting = connect(host, &HostAudioFormatController::countChanged, this, [&emitted]() { emitted++; });

    // Gated: the demo device has no host side, so the real backend is never read on it
    host->setBackendForTest(std::make_unique<FakeBackend>(fake), false);
    spin(800);
    check("QD18: a demo device doesn't read the host", fake->reads == 0 && !fake->watching);
    check("QD18: and uses its own report", host->hostChannels() == 0 && host->inputCount() == report && stripCount() == report);

    // Backend with nothing to say
    fake->endpoints.clear();
    host->setBackendForTest(std::make_unique<FakeBackend>(fake), true);
    check("QD18: the backend is read", waitFor(2000, [&]() { return fake->reads > 0; }));
    check("QD18: the backend watches", waitFor(2000, [&]() { return bool(fake->watching); }));
    spin(100);
    check("QD18: no endpoint, the device's report", host->hostChannels() == 0 && host->inputCount() == report && stripCount() == report);
    check("QD18: reads run off the UI thread", fake->offUiThread);

    // The host has eight while the device's report stays collapsed
    const int before = emitted;
    fake->set({ dspi(8) });
    check("QD18: a change reaches the count", waitFor(3000, [&]() { return host->hostChannels() == 8; }));
    check("QD18: eight strips over an idle report", host->inputCount() == 8 && stripCount() == 8
                                                    && b->activeInputChannels() == report);
    check("QD18: the leveller follows", leveller->property("inputCount").toInt() == 8);
    check("QD18: the matrix follows", matrix->property("inputCount").toInt() == 8);
    check("QD18: one signal for it", emitted == before + 1);
    qroot->grabWindow().save(m_out + QStringLiteral("/hostaudio-8-strips.png"));

    // Polling goes on, and nothing moves
    const int settled = emitted, readsSettled = fake->reads;
    spin(1000);
    check("QD18: no signal while nothing changes", emitted == settled);
    check("QD18: no reads while nothing changes", fake->reads == readsSettled);

    // Live: the user picks another format
    fake->set({ dspi(4) });
    check("QD18: the format changes live", waitFor(3000, [&]() { return host->inputCount() == 4 && stripCount() == 4; }));

    // The right unit of two
    const QString serial = b->selectedSerial();
    fake->set({ dspi(8, QStringLiteral("OTHER")), dspi(6, serial) });
    check("QD18: the selected unit's serial decides", waitFor(3000, [&]() { return host->inputCount() == 6; }));
    fake->set({ dspi(8), dspi(4) });
    check("QD18: two units and no serial, the device's report", waitFor(3000, [&]() { return host->hostChannels() == 0 && host->inputCount() == report; }));

    // A burst is one read
    const int readsBefore = fake->reads;
    for (int i = 0; i < 20; i++) fake->set({ dspi(8) });
    check("QD18: the burst lands", waitFor(3000, [&]() { return host->hostChannels() == 8; }));
    spin(700);
    check("QD18: a burst is read once or twice", fake->reads - readsBefore <= 2);

    // Non-USB sources don't follow the host
    const int source = b->inputSource();
    b->setInputSource(1);
    check("QD18: S/PDIF is stereo whatever the host says", waitFor(3000, [&]() { return b->inputSource() == 1 && host->inputCount() == 2 && stripCount() == 2; }));
    b->setInputSource(source);
    check("QD18: back on USB, the host again", waitFor(3000, [&]() { return b->inputSource() == source && host->inputCount() == 8; }));

    // The device goes: the backend stops and the host's answer goes with it
    fake->set({});
    check("QD18: the DSPi leaves the host's list, the device's report", waitFor(3000, [&]() { return host->hostChannels() == 0 && stripCount() == report; }));
    check("QD18: the matrix is back", matrix->property("inputCount").toInt() == matrixFallback);

    disconnect(counting);
    // As the app runs on the demo device: the real backend, gated
    host->setBackendForTest(makeBackend(), false);
    check("QD18: stops watching at the end", !fake->watching);
    check("QD18: and the real one isn't read here", host->hostChannels() == 0);
}

// On a hub's device, whose serial is no demo device's: the format on this host is
// not that device's
void SmokeDriver::testHostAudioOnHub() {
    auto *host = qobject_cast<HostAudioFormatController *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("hostAudio")).value<QObject *>());
    check("QD18: there on a hub session", host != nullptr);
    if (!host) return;
    auto fake = std::make_shared<FakeHost>();
    fake->endpoints = { dspi(8) };
    host->setBackendForTest(std::make_unique<FakeBackend>(fake), false);
    spin(800);
    check("QD18: a hub's device doesn't read this host", fake->reads == 0 && !fake->watching && host->hostChannels() == 0);
    host->setBackendForTest(HostAudio::makeBackend(), false);
}
