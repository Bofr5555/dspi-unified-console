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
//   "rail" (at the rail's top). The Modern shell reads them; another package
//   declares a further key with arrangement->declare().
// Until the Modern shell exists (WP LAY3), Modern shows the Classic shell.
class LayoutController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QString layout READ layout WRITE setLayout NOTIFY layoutChanged)
    Q_PROPERTY(bool modern READ modern NOTIFY layoutChanged)
    Q_PROPERTY(PrefsMap *arrangement READ arrangement CONSTANT)
public:
    static constexpr const char *kLayoutKey = "ui/layout";

    LayoutController(const AppContext &ctx, QObject *parent);
    static LayoutController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<LayoutController>(); }

    QString layout() const { return m_layout; }
    // "classic" or "modern"; anything else is ignored
    void setLayout(const QString &layout);
    bool modern() const { return m_layout == QLatin1String("modern"); }
    PrefsMap *arrangement() const { return m_arrangement; }

signals:
    void layoutChanged();

private:
    QString m_layout;
    PrefsMap *m_arrangement;
};

#endif // LAYOUTCONTROLLER_H
