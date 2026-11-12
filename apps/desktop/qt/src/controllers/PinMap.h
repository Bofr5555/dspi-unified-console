#ifndef PINMAP_H
#define PINMAP_H

#include <QList>
#include <QString>

#include "CoreApi.h"

// Who holds each GPIO, for Settings > Overview (the Mac's pinAssignment and
// the core's claims_from_state, dspi-session pins.rs): the firmware's own
// rules, so a feature that is only configured holds nothing.
namespace PinMap {

// The roles the Overview groups by, in its order
enum Role { Outputs = 0, Clocks, Inputs, Control, Other };

struct Claim {
    int pin;
    int role;
    QString label;
};

// A live control-surface binding's GPIO (dspi_cs_pin_uses)
struct SurfacePin {
    int pin;
    int slot;
};

// Every GPIO the device offers: 0 to 22 and 26 to 28 (23 to 25 are power and
// the LED), on both platforms.
QList<int> gpioPins();

// The claims on valid pins, one per pin (the first claim wins, in the
// firmware's order), sorted by pin.
QList<Claim> claims(const DspState &state, const QList<SurfacePin> &surfaces);

} // namespace PinMap

#endif // PINMAP_H
