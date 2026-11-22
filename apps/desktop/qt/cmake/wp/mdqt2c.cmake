# -- WP MDQT2c -- The job controllers over several open devices: Measure and
# Tune are facades over the active device's MeasureSession and TuneSession; the
# others keep their per-device state in the session's stash (SessionState.h)
list(APPEND DSPI_APP_SOURCES
    src/controllers/MeasureSession.h src/controllers/MeasureSession.cpp
    src/controllers/TuneSession.h src/controllers/TuneSession.cpp)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeJobs.cpp)
