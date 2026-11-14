# -- WP HUBADMIN -- Settings > Installer edits the hub's copy over DSPi Link:
# the smoke section against `uconsole --demo hub` (LimitsController and
# InstallerPage.qml are already listed)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeHubAdmin.cpp)
