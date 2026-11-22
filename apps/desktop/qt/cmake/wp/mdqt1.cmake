# -- WP MDQT1 -- Several open devices: the registry, a session each, and the
# Link session of each (LinkController is the active one's facade)
list(APPEND DSPI_APP_SOURCES
    src/core/DeviceRegistry.h src/core/DeviceRegistry.cpp
    src/core/DeviceSession.h src/core/DeviceSession.cpp
    src/core/SessionModel.h src/core/SessionModel.cpp
    src/controllers/LinkSession.h src/controllers/LinkSession.cpp)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeRegistry.cpp)
