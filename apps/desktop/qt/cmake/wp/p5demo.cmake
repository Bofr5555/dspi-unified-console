# -- WP P5DEMO -- The spike on the demo DSPi with its simulated microphone
# (--measure-spike-sim; the core's measure-sim feature, on in dspi-capi): the
# smoke section p5demo-sim reaches a timing verdict with no capture opened.
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeMeasureSim.cpp)
