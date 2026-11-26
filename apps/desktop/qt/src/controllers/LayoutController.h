#ifndef LAYOUTCONTROLLER_H
#define LAYOUTCONTROLLER_H

#include <QObject>
#include <QString>
#include <QtQml/qqmlregistration.h>

#include "PrefsMap.h"
#include "Registrar.h"

class QQmlEngine;
class QJSEngine;

// -- WP LAY0 -- Which shell the main window shows (MODLAY 4.6): Classic (the
// Mac console's menu bar, sidebar and tool windows) or Modern (a rail, pages
// with tabs, an inspector). MainWindow loads the shell of `layout`; a switch
// unloads one shell and loads the other in the same window, so the device
// session, the selection and the open tool windows stay (they live in the
// controllers and in MainWindow).
//
// - `layout` is "classic" or "modern", stored under ui/layout. With nothing
//   stored, a person who finished Getting Started in an earlier version
//   (onboarding/setupDone) keeps Classic and a new install gets Modern
//   (owner, 2026-11-21); the choice is stored at once, so it never flips.
// - `arrangement` is how Modern is arranged (ui/arrangement/*), so the owner
//   can A/B it live: `navigation` "rail" (variant B: a rail and tabs in each
//   page, the default) or "tabs" (variant A: top tabs), and `deviceTabs`
//   "topBar" (the open devices as tabs in the top bar, the default) or
//   "rail" (at the rail's top). -- WP LAY4 -- `tone` "grey" (the default) or
//   "darker" (Theme's surfaces in Modern), `inspector` "shown" (the default)
//   or "hidden", `bandBar` "hidden" (the default) or "shown" (the band bar
//   docked under the Equaliser's graph). The Modern shell reads them; another package
//   declares a further key with arrangement->declare().
class LayoutController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QString layout READ layout WRITE setLayout NOTIFY layoutChanged)
    Q_PROPERTY(bool modern READ modern NOTIFY layoutChanged)
    // -- WP RBFIX -- Why the layout cannot be switched now ("" when it can):
    // leaving Modern closes the Measure page, which stops every measurement
    Q_PROPERTY(QString switchBlocker READ switchBlocker NOTIFY switchBlockerChanged)
    Q_PROPERTY(PrefsMap *arrangement READ arrangement CONSTANT)
    // -- WP LAY3 -- "system" (the default), "light" or "dark" (ui/theme): the
    // colour scheme of both layouts. DSPI_COLOR_SCHEME wins at the start.
    Q_PROPERTY(QString theme READ theme WRITE setTheme NOTIFY themeChanged)
    // -- WP UB13 -- (UXP B13, D11) Settings > Appearance > Accessibility, both
    // off on a mouse desktop. `outlineControls` (ui/outlineControls, false): a
    // 1 px line of Theme.controlOutline round fields, combo boxes, steppers,
    // buttons, segments and chips, for those who cannot tell them from the page
    // by tone alone. `density` (ui/density): "automatic" (the default),
    // "standard" or "comfortable"; `comfortable` is what it comes to, automatic
    // being comfortable on a computer with a touch screen (`touchScreen`, read at
    // the start; DSPI_TOUCH_SCREEN=1|0 says it instead). Theme applies the
    // density in Modern only: Classic keeps the Mac console's sizes.
    Q_PROPERTY(bool outlineControls READ outlineControls WRITE setOutlineControls NOTIFY outlineControlsChanged)
    Q_PROPERTY(QString density READ density WRITE setDensity NOTIFY densityChanged)
    Q_PROPERTY(bool touchScreen READ touchScreen CONSTANT)
    Q_PROPERTY(bool comfortable READ comfortable NOTIFY densityChanged)
public:
    static constexpr const char *kLayoutKey = "ui/layout";
    static constexpr const char *kThemeKey = "ui/theme";
    static constexpr const char *kOutlineKey = "ui/outlineControls";   // -- WP UB13 --
    static constexpr const char *kDensityKey = "ui/density";           // -- WP UB13 --

    LayoutController(const AppContext &ctx, QObject *parent);
    static LayoutController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<LayoutController>(); }

    QString layout() const { return m_layout; }
    // "classic" or "modern"; anything else is ignored. -- WP RBFIX -- Refused
    // with a notice while there is a switchBlocker
    void setLayout(const QString &layout);
    bool modern() const { return m_layout == QLatin1String("modern"); }
    QString switchBlocker() const;
    PrefsMap *arrangement() const { return m_arrangement; }
    QString theme() const { return m_theme; }
    // "system", "light" or "dark"; anything else is ignored
    void setTheme(const QString &theme);
    // -- WP UB13 --
    bool outlineControls() const { return m_outlineControls; }
    void setOutlineControls(bool on);
    QString density() const { return m_density; }
    // "automatic", "standard" or "comfortable"; anything else is ignored
    void setDensity(const QString &density);
    bool touchScreen() const { return m_touchScreen; }
    bool comfortable() const { return comfortableFor(m_density, m_touchScreen); }
    static bool comfortableFor(const QString &density, bool touchScreen) {
        return density == QLatin1String("comfortable") || (density == QLatin1String("automatic") && touchScreen);
    }

signals:
    void layoutChanging();   // -- WP LAY3 -- the loaded shell is about to go
    void layoutChanged();
    void switchBlockerChanged();   // -- WP RBFIX --
    void themeChanged();   // -- WP LAY3 --
    void outlineControlsChanged();   // -- WP UB13 --
    void densityChanged();           // -- WP UB13 --

private:
    AppContext m_ctx;
    QString m_layout;
    QString m_theme = QStringLiteral("system");
    PrefsMap *m_arrangement;
    bool m_outlineControls = false;                         // -- WP UB13 --
    QString m_density = QStringLiteral("automatic");        // -- WP UB13 --
    bool m_touchScreen = false;                             // -- WP UB13 --
};

#endif // LAYOUTCONTROLLER_H
