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
public:
    static constexpr const char *kLayoutKey = "ui/layout";
    static constexpr const char *kThemeKey = "ui/theme";

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

signals:
    void layoutChanging();   // -- WP LAY3 -- the loaded shell is about to go
    void layoutChanged();
    void switchBlockerChanged();   // -- WP RBFIX --
    void themeChanged();   // -- WP LAY3 --

private:
    AppContext m_ctx;
    QString m_layout;
    QString m_theme = QStringLiteral("system");
    PrefsMap *m_arrangement;
};

#endif // LAYOUTCONTROLLER_H
