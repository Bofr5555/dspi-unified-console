// -- FIX016 -- The 0.16 review's Qt findings (REVG), each checked where it was
// found ("fix016-qt", on the demo, last in the run; everything put back):
// - R1-QT: the demo's simulated microphone, once chosen, is the demo's again
//   when the demo is opened again (ua2 after mcp5-measure and ua5-settings).
#include "SmokeDriver.h"

#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "MeasureController.h"
#include "Registrar.h"

namespace {

// ---- R1-QT: the simulated microphone comes back with the demo
void micWithTheDemo(SmokeDriver &d) {
    auto *mc = Registrar::instance<MeasureController>();
    DeviceModel *device = d.app().device;
    if (!mc) {
        d.check("fix016 R1-QT: the Measure controller", false);
        return;
    }
    if (!device->connected() || !device->demo()) device->openDemo();
    d.waitFor(10000, [&] { return device->connected() && device->demo(); });
    d.waitIdle();
    mc->panelShown(true);   // the measurement thread, as the Measure view starts it
    mc->setMicChoice(QStringLiteral("sim"));   // the person picks the simulated one
    d.check("fix016 R1-QT: the demo's simulated microphone is chosen",
            d.waitFor(5000, [&] { return mc->simAvailable() && mc->micChoice() == QLatin1String("sim"); }));
    // The demo closes (as Settings' and the reconnect paths do) and opens again
    d.app().core->worker()->post([](FfiCore *c) { dspi_disconnect(c); });
    device->refresh();
    d.waitFor(5000, [&] { return !device->connected(); });
    d.waitFor(3000, [&] { return !mc->simAvailable(); });
    device->openDemo();
    d.waitFor(10000, [&] { return device->connected() && device->demo(); });
    d.waitIdle();
    d.check(QStringLiteral("fix016 R1-QT: the demo opened again measures with its simulated microphone again (%1)").arg(mc->micChoice()),
            d.waitFor(5000, [&] { return mc->simAvailable() && mc->micChoice() == QLatin1String("sim"); }));
    mc->panelShown(false);
}

void fix016Section(SmokeDriver &d) {
    micWithTheDemo(d);
}

const SmokeDriver::Section fix016("fix016-qt", 1000, &fix016Section);

} // namespace
