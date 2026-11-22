# -- WP LAY4 -- The Equaliser's pages (PEQ, Crossover, Delay & Gain), the channel
# strip and the inspector
list(APPEND DSPI_QML_FILES
    qml/modern/Inspector.qml qml/modern/InspectorState.qml qml/modern/ChannelInspector.qml qml/modern/BandInspector.qml
    qml/modern/ChannelStrip.qml qml/modern/ChannelInfo.qml qml/modern/PairLink.qml qml/modern/OutputEnableQuestion.qml
    qml/modern/pages/EqualiserPeq.qml qml/modern/pages/EqualiserCrossover.qml qml/modern/pages/EqualiserCrossoverSide.qml
    qml/modern/pages/EqualiserDelay.qml qml/modern/pages/EqualiserDelayRow.qml qml/modern/pages/EqualiserGraphToggles.qml
    qml/modern/pages/EqualiserTools.qml)
list(APPEND DSPI_QML_SINGLETONS qml/modern/InspectorState.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeLay4.cpp)
