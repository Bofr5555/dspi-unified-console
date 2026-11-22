#ifndef HISTORYCONTROLLER_H
#define HISTORYCONTROLLER_H

#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// -- WP QP18 -- Edit > Undo, Edit > Redo and Edit > History, from the core's
// change journal (dspi_undo, dspi_redo, dspi_can_undo, dspi_can_redo,
// dspi_history). The core decides what may be undone and steps over the rest;
// an undo or redo is a write like any other (DeviceModel::write), so a
// read-only device refuses it. When another writer changed a value since,
// the core leaves it as they set it and says who: that sentence becomes a
// notice (NoticeCenter), as does a skipped change.
//
// The state is read again after every snapshot (a write of this app's own
// raises no notification) and on NOTIFY_HISTORY.
//
// -- WP MDQT2b -- Each open device has its own journal (its core's): the
// window shows the active one's, read again on a switch; a read asked of the
// one left is dropped.
class HistoryController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool canUndo READ canUndo NOTIFY changed)
    Q_PROPERTY(bool canRedo READ canRedo NOTIFY changed)
    // Newest first: [{ title, detail, source, time, undoable, agent }]; -- WP MCP4 --
    // `agent` is who made it when it was an AI agent ("AI · Claude Code"), else ""
    Q_PROPERTY(QVariantList entries READ entries NOTIFY changed)
    // The last undo's or redo's note, "" when it had none
    Q_PROPERTY(QString lastNote READ lastNote NOTIFY moved)
public:
    // How many changes the History window lists
    static constexpr int kShown = 50;

    HistoryController(const AppContext &ctx, QObject *parent = nullptr);
    static HistoryController *create(QQmlEngine *, QJSEngine *);

    bool canUndo() const { return m_canUndo; }
    bool canRedo() const { return m_canRedo; }
    QVariantList entries() const { return m_entries; }
    QString lastNote() const { return m_lastNote; }

    Q_INVOKABLE void undo();
    Q_INVOKABLE void redo();
    // Reads the journal again (on the worker)
    Q_INVOKABLE void reload();

    // A journal entry in words: { title, detail } (tests)
    static QVariantMap describe(const QJsonObject &entry);

signals:
    void changed();
    // After an undo or redo: whether it went through and what it moved ("" for nothing)
    void moved(bool ok, const QString &title);

private:
    void step(bool forward);

    AppContext m_ctx;
    bool m_canUndo = false;
    bool m_canRedo = false;
    QVariantList m_entries;
    QString m_lastNote;
    bool m_reading = false;
    bool m_again = false;
    int m_steps = 0;
};

#endif // HISTORYCONTROLLER_H
