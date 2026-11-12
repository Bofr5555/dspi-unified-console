#include "PinMap.h"

#include <QCoreApplication>
#include <algorithm>

namespace PinMap {

namespace {

constexpr uint8_t kNoPin = 0xFF;   // DAC mute and ADAT input: no GPIO set

class Builder {
public:
    void claim(int pin, int role, const QString &label) {
        if (!valid(pin)) return;
        for (const Claim &c : std::as_const(list))
            if (c.pin == pin) return;
        list.append({ pin, role, label });
    }
    static bool valid(int pin) { return (pin >= 0 && pin <= 22) || (pin >= 26 && pin <= 28); }
    QList<Claim> list;
};

} // namespace

QList<int> gpioPins() {
    QList<int> pins;
    for (int p = 0; p <= 28; ++p)
        if (Builder::valid(p)) pins.append(p);
    return pins;
}

QList<Claim> claims(const DspState &s, const QList<SurfacePin> &surfaces) {
    Builder b;

    // The output slots, then the PDM sub as the last entry
    const int count = std::min<int>(s.num_pin_outputs, MAX_PHYSICAL_OUTPUTS);
    for (int i = 0; i < count; ++i) {
        if (i + 1 < count) b.claim(s.output_pins[i], Outputs, QCoreApplication::translate("PinMap", "Output %1").arg(i + 1));
        else b.claim(s.output_pins[i], Outputs, QCoreApplication::translate("PinMap", "Subwoofer"));
    }

    // Clocks: LRCLK is always BCK + 1; the slave pair only in split mode
    b.claim(s.i2s_bck_pin, Clocks, QCoreApplication::translate("PinMap", "I2S BCK"));
    b.claim(s.i2s_bck_pin + 1, Clocks, QCoreApplication::translate("PinMap", "I2S LRCLK"));
    if (s.i2s_clock_pin_mode == 1 && s.i2s_bck_pin_slave != 0) {
        b.claim(s.i2s_bck_pin_slave, Clocks, QCoreApplication::translate("PinMap", "I2S Slave BCK"));
        b.claim(s.i2s_bck_pin_slave + 1, Clocks, QCoreApplication::translate("PinMap", "I2S Slave LRCLK"));
    }
    if (s.mck_enabled) b.claim(s.mck_pin, Clocks, QCoreApplication::translate("PinMap", "I2S MCK"));

    // Inputs: S/PDIF 1 always, the optional ones (bits 1 to 3) while enabled
    const bool optionalSpdif = (s.spdif_inputs_enabled & 0x0E) != 0;
    b.claim(s.spdif_rx_pins[0], Inputs, optionalSpdif ? QCoreApplication::translate("PinMap", "S/PDIF 1 RX") : QCoreApplication::translate("PinMap", "S/PDIF RX"));
    for (int i = 1; i < 4; ++i)
        if (s.spdif_inputs_enabled & (1u << i)) b.claim(s.spdif_rx_pins[i], Inputs, QCoreApplication::translate("PinMap", "S/PDIF %1 RX").arg(i + 1));
    // The I2S input's active pairs
    const int pairs = std::clamp(std::max<int>(s.i2s_input_channels, 2) / 2, 1, 4);
    for (int pair = 0; pair < pairs; ++pair)
        b.claim(s.i2s_rx_pins[pair], Inputs, pairs > 1 ? QCoreApplication::translate("PinMap", "I2S RX %1").arg(pair + 1) : QCoreApplication::translate("PinMap", "I2S RX"));
    if (s.adat_input_enabled && s.adat_input_pin != kNoPin) b.claim(s.adat_input_pin, Inputs, QCoreApplication::translate("PinMap", "ADAT Input"));

    if (s.adat_out_enabled) b.claim(s.adat_out_pin, Outputs, QCoreApplication::translate("PinMap", "ADAT Output"));
    if (s.dac_mute_enabled && s.dac_mute_pin != kNoPin) b.claim(s.dac_mute_pin, Other, QCoreApplication::translate("PinMap", "DAC Mute"));

    // Control: the interfaces while they are up, the live surface bindings
    if (s.ctrl_status.uart_live) {
        b.claim(s.uart.tx_pin, Control, QCoreApplication::translate("PinMap", "UART Control"));
        b.claim(s.uart.rx_pin, Control, QCoreApplication::translate("PinMap", "UART Control"));
    }
    if (s.ctrl_status.i2c_live) {
        b.claim(s.i2c.sda_pin, Control, QCoreApplication::translate("PinMap", "I2C Control"));
        b.claim(s.i2c.scl_pin, Control, QCoreApplication::translate("PinMap", "I2C Control"));
    }
    for (const SurfacePin &sp : surfaces) b.claim(sp.pin, Control, QCoreApplication::translate("PinMap", "Control Surface %1").arg(sp.slot + 1));

    std::sort(b.list.begin(), b.list.end(), [](const Claim &a, const Claim &c) { return a.pin < c.pin; });
    return b.list;
}

} // namespace PinMap
