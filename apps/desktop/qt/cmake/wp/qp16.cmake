# -- WP QP16 -- Firmware and read-only: the firmware window and its installer,
# the bootloader locator, the mismatch, read-only and delay-refusal banners,
# and the device-access (udev) dialog
list(APPEND DSPI_APP_SOURCES
    src/controllers/FirmwareController.h src/controllers/FirmwareController.cpp
    src/controllers/ReadOnlyController.h src/controllers/ReadOnlyController.cpp
    src/platform/BootloaderLocator.h src/platform/BootloaderLocator.cpp
    src/platform/DeviceAccess.h src/platform/DeviceAccess.cpp)
list(APPEND DSPI_QML_FILES
    qml/tools/FirmwarePanel.qml   # -- WP LAY1b -- was FirmwareWindow.qml
    qml/banners/FirmwareBanner.qml
    qml/banners/ReadOnlyBanner.qml
    qml/banners/DelayRefusalBanner.qml
    qml/platform/UdevDialog.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeFirmware.cpp smoke/SmokeReadOnly.cpp)

# The firmware this app carries (its version is the one it expects a device to
# run), as resources under :/firmware on each target
function(dspi_qp16_target target)
    file(GLOB DSPI_QP16_IMAGES CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/firmware/*.uf2")
    qt_add_resources(${target} dspi_qp16_firmware
        PREFIX "/firmware"
        BASE "${CMAKE_CURRENT_SOURCE_DIR}/firmware"
        FILES ${DSPI_QP16_IMAGES})
endfunction()
list(APPEND DSPI_APP_HOOKS dspi_qp16_target)
