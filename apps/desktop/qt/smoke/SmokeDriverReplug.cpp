// The replug pass of the smoke driver (DSPI_SMOKE_HW=replug, see run-replug.sh):
// the real app, with the owner unplugging and replugging the device three times.
// It writes nothing. The app's views stay as they are, so a Dashboard's analyser
// overlay runs across the unplug, as it does for a user.
//
// sysfs tells when the device really left and came back, so the app's reaction is
// timed from the event and not from the owner's prompt.

#include "SmokeDriver.h"

#include "DSPiBridge.h"

#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QImage>
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <cstdio>

static const int kCycles = 3;
static const int kOwnerMs = 45000;        // run.sh's overall timeout allows for six of these
static const int kDisconnectedMs = 2500;  // the hotplug poll is 500 ms, a read fails at once
static const int kReconnectedMs = 15000;  // the firmware's boot, enumeration, the connect sequence

// Whether the device with this serial is on the bus, by sysfs alone: nothing opens it
static bool onBus(const QString &serial) {
    const QString base = QStringLiteral("/sys/bus/usb/devices/");
    for (const QString &d : QDir(base).entryList(QDir::Dirs | QDir::NoDotAndDotDot)) {
        QFile f(base + d + QStringLiteral("/serial"));
        if (f.open(QIODevice::ReadOnly) && QString::fromLatin1(f.readAll()).trimmed() == serial) return true;
    }
    return false;
}

static void prompt(const QString &text) {
    printf("smoke:\nsmoke: >>> %s\n", qPrintable(text));
    fflush(stdout);
}

bool SmokeDriver::replugCycle(int cycle, const QString &serial, Baseline *want) {
    auto *b = m_.bridge;
    auto *main = qobject_cast<QQuickWindow *>(root());
    const QString name = QStringLiteral("cycle %1/%2").arg(cycle).arg(kCycles);
    const int failuresBefore = m_failures;
    const auto grab = [&](const char *what) {
        if (main) main->grabWindow().save(m_out + QStringLiteral("/cycle-%1-%2.png").arg(cycle).arg(QLatin1String(what)));
    };

    prompt(QStringLiteral("%1: UNPLUG the DSPi now (%2 s)").arg(name).arg(kOwnerMs / 1000));
    if (!waitFor(kOwnerMs, [&serial]() { return !onBus(serial); })) {
        fail(name + ": unplug", "the device stayed on the bus");
        return false;
    }
    printf("smoke: unplug seen on the bus\n");
    fflush(stdout);
    QElapsedTimer t;
    t.start();
    const bool gone = waitFor(kDisconnectedMs, [b]() { return !b->connected(); });
    outcome(name + ": disconnected state", gone, QStringLiteral("%1 ms after the bus (limit %2)").arg(t.elapsed()).arg(kDisconnectedMs));
    spin(1000);   // it stays that way, and the app keeps running with no device
    outcome(name + ": stays disconnected, nothing selected", !b->connected() && b->selectedSerial().isEmpty(),
            b->selectedSerial());
    grab("unplugged");

    prompt(QStringLiteral("%1: PLUG the DSPi back in now (%2 s)").arg(name).arg(kOwnerMs / 1000));
    if (!waitFor(kOwnerMs, [&serial]() { return onBus(serial); })) {
        fail(name + ": replug", "the device did not come back on the bus");
        return false;
    }
    printf("smoke: replug seen on the bus\n");
    fflush(stdout);
    t.restart();
    const bool back = waitFor(kReconnectedMs, [b, &serial]() {
        return b->connected() && b->selectedSerial() == serial && b->compat() == COMPAT_OK;
    });
    outcome(name + ": auto-reconnect", back, QStringLiteral("%1 ms after the bus (limit %2); %3").arg(t.elapsed()).arg(kReconnectedMs).arg(b->connectionError()));
    if (!back) return false;
    spin(800);    // the connection settles; the status timer runs on it
    SystemStatus status {};
    outcome(name + ": status reads", dspi_fetch_status(b->core(), &status));
    outcome(name + ": no connection error", b->connectionError().isEmpty(), b->connectionError());
    const bool same = compareBaseline(*want, "after the replug");
    if (same) {
        pass(name + ": image identical");
    } else {
        // Only m_failures counted so far. If the replug cut the device's power, a
        // reboot drops whatever RAM held unsaved: the next cycle starts from here.
        printf("smoke: a power cycle drops unsaved changes: was anything unsaved when this run began?\n");
        fflush(stdout);
        if (!readBaseline(want)) fail(name + ": new baseline", "the image could not be read");
    }
    grab("replugged");
    const bool ok = m_failures == failuresBefore;
    printf("smoke: ==== %s: %s ====\n", qPrintable(name), ok ? "PASS" : "FAIL");
    fflush(stdout);
    return true;
}

void SmokeDriver::runReplug() {
    printf("smoke: replug pass: %d cycles, each step waits %d s for you\n", kCycles, kOwnerMs / 1000);
    if (!hwPreflight()) { finish(); return; }
    const QString serial = m_.bridge->selectedSerial();
    if (!onBus(serial)) {
        fail("sysfs", "the connected device is not in /sys/bus/usb/devices: this container has no real sysfs");
        finish();
        return;
    }
    Baseline want;
    if (!readBaseline(&want)) {
        fail("baseline", "the device's image could not be read");
        finish();
        return;
    }
    pass("baseline", QStringLiteral("%1 image bytes").arg(want.image.size()));
    for (int c = 1; c <= kCycles; c++) {
        if (!replugCycle(c, serial, &want)) {
            printf("smoke: ==== cycle %d/%d: FAIL, stopping here ====\n", c, kCycles);
            fflush(stdout);
            break;
        }
    }
    finish();
}
