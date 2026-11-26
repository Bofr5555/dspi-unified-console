# -- INTC -- the integration of DESREV wave C: the folds' shared pieces.
# FloatingBackground: the floating surface of popovers (DL5 fold).
list(APPEND DSPI_QML_FILES qml/components/FloatingBackground.qml)
# DeviceListRow: the device rows Devices › This Computer and the switcher share (DL5 fold).
list(APPEND DSPI_QML_FILES qml/devices/DeviceListRow.qml)
# BodeTicks: the axis ticks of every Bode graph in Modern (DL8 fold).
list(APPEND DSPI_QML_FILES qml/components/BodeTicks.qml)
list(APPEND DSPI_QML_SINGLETONS qml/components/BodeTicks.qml)
