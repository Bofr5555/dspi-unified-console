# -- WP LAY5a -- Modern's Overview, Routing, Effects and Analyse pages
list(APPEND DSPI_QML_FILES
    qml/modern/InspectorButton.qml
    qml/modern/pages/OverviewSummary.qml qml/modern/pages/OverviewTools.qml
    qml/modern/pages/RoutingInspector.qml qml/modern/pages/RoutingTools.qml
    qml/modern/pages/EffectsState.qml qml/modern/pages/EffectsChain.qml qml/modern/pages/EffectsTools.qml
    qml/modern/pages/AnalyseTools.qml)
list(APPEND DSPI_QML_SINGLETONS qml/modern/pages/EffectsState.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeLay5a.cpp)
