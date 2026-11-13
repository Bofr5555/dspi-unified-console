#ifndef SHORTCUTCONTROLLER_H
#define SHORTCUTCONTROLLER_H

#include <QObject>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP QP18 -- The Device menu's keyboard rows the Mac does not have (owner,
// 2026-11-12; PRO1 U3): Ctrl+1 to Ctrl+9 recall presets 1 to 9 (through the
// sidebar's switch, so unsaved changes are asked about first), Ctrl+M mutes
// and unmutes the volume, Ctrl+B switches the master EQ bypass. Commit
// (Ctrl+S) and Revert (Ctrl+Shift+R) are FilesController's rows with a
// shortcut added.
class ShortcutController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool muted READ muted NOTIFY changed)
    Q_PROPERTY(bool bypassed READ bypassed NOTIFY changed)
public:
    ShortcutController(const AppContext &ctx, QObject *parent = nullptr);
    static ShortcutController *create(QQmlEngine *, QJSEngine *);

    bool muted() const;
    bool bypassed() const;

    // Preset `number` (1 to 9)
    Q_INVOKABLE void recallPreset(int number);
    Q_INVOKABLE void toggleMute();
    Q_INVOKABLE void toggleBypass();

signals:
    void changed();

private:
    AppContext m_ctx;
};

#endif // SHORTCUTCONTROLLER_H
