#ifndef DEMODEVICE_H
#define DEMODEVICE_H

// The demo device's serial: FakeDevice's in crates/dspi-transport/src/fake.rs.
// dspi_open_demo opens it whatever the USB bus holds.
namespace DemoDevice {
inline constexpr char kSerial[] = "DEMO0000DSPI0001";
}

#endif // DEMODEVICE_H
