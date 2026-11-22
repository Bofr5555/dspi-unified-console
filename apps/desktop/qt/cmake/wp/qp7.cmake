# -- WP QP7 -- Effects A: crossfeed (with the pair map), loudness, leveller, psychoacoustic bass
list(APPEND DSPI_APP_SOURCES
    src/controllers/EffectsController.h src/controllers/EffectsController.cpp
    src/controllers/ExtController.h src/controllers/ExtController.cpp)
list(APPEND DSPI_QML_FILES
    qml/tools/EffectHeader.qml qml/tools/EffectParameterRow.qml qml/tools/EffectChip.qml
    qml/tools/EffectMenuButton.qml qml/tools/EffectGraph.qml qml/tools/EffectSection.qml
    qml/tools/EffectRule.qml qml/tools/EffectColumns.qml
    qml/tools/CrossfeedPairMap.qml)   # -- WP LAY1a -- the windows are panels (lay1a.cmake)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeEffects.cpp)
