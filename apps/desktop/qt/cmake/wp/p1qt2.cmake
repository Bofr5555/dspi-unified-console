# -- WP P1QT2 -- Settings > Installer (limits, the PIN and the lock, the
# locked-user banner and its one unlock prompt), confirmation tokens for the
# dangerous operations, the AutoEQ preview and channel pair, the core's notice
# table, undo groups (PLAN-CLIENTS 5.9)
list(APPEND DSPI_APP_SOURCES
    src/controllers/LimitsController.h src/controllers/LimitsController.cpp
    src/controllers/Confirm.h src/controllers/Confirm.cpp)
list(APPEND DSPI_QML_FILES
    qml/settings/pages/InstallerPage.qml
    qml/installer/LimitField.qml
    qml/installer/LockBanner.qml
    qml/installer/UnlockDialog.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeInstaller.cpp)
