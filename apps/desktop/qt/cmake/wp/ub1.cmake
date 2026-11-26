# -- WP UB1 -- (UXP B1, owner D5) crossovers and routing from the confirmed
# speaker layout: the controller, the sheet, the smoke section ub1-layout
list(APPEND DSPI_APP_SOURCES src/controllers/LayoutSetupController.h src/controllers/LayoutSetupController.cpp)
list(APPEND DSPI_QML_FILES qml/modern/pages/CrossoverFromLayout.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeUb1.cpp)
