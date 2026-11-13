# Work packages' file lists

One file per work package, `<wp>.cmake` (`qp1.cmake`, `qp13.cmake`, ...),
included by `../../CMakeLists.txt` in name order. It only appends to the lists:

```cmake
# -- WP QP13 -- Devices and Link
list(APPEND DSPI_APP_SOURCES
    src/controllers/DevicesController.h src/controllers/DevicesController.cpp)
list(APPEND DSPI_QML_FILES qml/devices/DevicesPopup.qml qml/devices/PairingDialog.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeDevices.cpp)
```

Lists: `DSPI_APP_SOURCES`, `DSPI_QML_FILES`, `DSPI_QML_SINGLETONS` (QML files
that are `pragma Singleton`), `DSPI_SMOKE_SOURCES`, `DSPI_SMOKE_QML_FILES`.
Paths are relative to `apps/desktop/qt`.
