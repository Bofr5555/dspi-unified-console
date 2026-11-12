#ifndef MONITORCONTROLLER_H
#define MONITORCONTROLLER_H

#include <QAbstractListModel>
#include <QObject>
#include <QStringList>
#include <QtQml/qqmlregistration.h>
#include <atomic>
#include <cstdint>
#include <memory>

#include "Poller.h"
#include "Registrar.h"

class QQmlEngine;
class QJSEngine;

// -- WP QP9 -- The Interrupt Monitor's rows: one line per notification, oldest
// first, at most kLimit. Role `line`, the shown text.
class MonitorLines : public QAbstractListModel {
    Q_OBJECT
    QML_ANONYMOUS
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)
public:
    static constexpr int kLimit = 2000;
    enum Role { LineRole = Qt::UserRole + 1 };

    using QAbstractListModel::QAbstractListModel;
    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    // Appends `lines` and drops the oldest past kLimit
    void append(const QStringList &lines);
    void clear();
    const QStringList &lines() const { return m_lines; }

signals:
    void countChanged();

private:
    QStringList m_lines;
};

// -- WP QP9 -- The Interrupt Monitor (the Mac's InterruptMonitor): the core
// keeps a line for every notification it drains (dspi_monitor_read), whether
// or not the window is open; this controller shows them.
//
// - While the window shows and the monitor is not paused, a poll reads the new
//   lines every kPollMs (the Mac publishes its log every 40 ms). Opening the
//   window catches up from the last line read, so the first open shows what
//   the core still keeps (up to 2000 lines).
// - Pause drops what arrives meanwhile, as the Mac does: on Resume the lines
//   logged while paused are skipped. Pause lives here, so it survives closing
//   the window.
// - Clear empties the rows and the core's log (ids go on rising, so no line
//   is read twice).
// - Copy puts every row on the clipboard, one per line.
class MonitorController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(MonitorLines *lines READ lines CONSTANT)
    Q_PROPERTY(bool paused READ paused WRITE setPaused NOTIFY pausedChanged)
    Q_PROPERTY(bool windowShown READ windowShown WRITE setWindowShown NOTIFY windowShownChanged)
    // "inactive" without a device, else "paused" or "listening"
    Q_PROPERTY(QString status READ status NOTIFY statusChanged)
public:
    static constexpr int kPollMs = 50;
    // One read's buffer; the core writes whole lines only
    static constexpr int kReadBytes = 64 * 1024;

    MonitorController(const AppContext &ctx, QObject *parent = nullptr);
    static MonitorController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<MonitorController>(); }

    MonitorLines *lines() const { return m_lines; }
    bool paused() const { return m_paused; }
    void setPaused(bool paused);
    bool windowShown() const { return m_shown; }
    void setWindowShown(bool shown);
    QString status() const;

    Q_INVOKABLE void togglePause() { setPaused(!m_paused); }
    Q_INVOKABLE void clear();
    Q_INVOKABLE void copyAll() const;
    // The text Copy puts on the clipboard
    Q_INVOKABLE QString text() const;

    // A core line ("<ms>\t<text>") as shown: local "HH:mm:ss.zzz", two spaces,
    // the text. A line without a time is shown as it is.
    static QString shown(const QString &coreLine);
    // Polls, for tests
    Poller::Id poll() const { return m_poll; }

signals:
    void pausedChanged();
    void windowShownChanged();
    void statusChanged();
    // New rows arrived (the view scrolls to the end)
    void appended();

private:
    struct Batch {
        uint64_t last = 0;
        QStringList lines;
    };
    static Batch readAfter(FfiCore *core, uint64_t after);
    void apply(const Batch &batch);
    void skipToEnd();
    void updatePoll();

    AppContext m_ctx;
    MonitorLines *m_lines;
    Poller::Id m_poll = 0;
    // The last line read; the poll reads it on the worker
    std::shared_ptr<std::atomic<uint64_t>> m_last = std::make_shared<std::atomic<uint64_t>>(0);
    bool m_paused = false;
    bool m_shown = false;
    // A Resume's skip to the log's end is under way: batches until then are dropped
    bool m_skipping = false;
};

#endif // MONITORCONTROLLER_H
