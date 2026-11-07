#ifndef AUTOEQLIBRARY_H
#define AUTOEQLIBRARY_H

#include <QAbstractListModel>
#include <QHash>
#include <QList>
#include <QSet>
#include <QTimer>
#include <QUrl>
#include <QVariantList>
#include <QVector>
#include <atomic>
#include <functional>
#include <memory>

class DSPiBridge;
class QThread;
struct AutoEqRebuildHandle;
struct AutoEqFetchContext;

// What one GET came to: the server's answer, whatever its status, or why there
// was none
struct AutoEqFetchAnswer {
    bool answered = false;
    int status = 0;
    QByteArray body;
    qint64 retryAfterS = -1, rateLimitRemaining = -1, rateLimitReset = -1;  // -1: no such header
    QString failure;
};

// One AutoEQ headphone profile: a preamp and up to ten parametric filters
struct AutoEqEntry {
    QString id, manufacturer, model, source, formFactor;
    double preamp = 0;
    struct Filter { QString type; double freq, q, gain; };
    QVector<Filter> filters;
    QString displayName() const { return model.isEmpty() ? manufacturer : manufacturer + " " + model; }
};

// The browser's rows: the database filtered by the search text
class AutoEqModel : public QAbstractListModel
{
    Q_OBJECT
public:
    enum Role { IdRole = Qt::UserRole + 1, NameRole, SourceRole, SourceNameRole, FormFactorRole, FavoriteRole };
    explicit AutoEqModel(QObject *parent = nullptr) : QAbstractListModel(parent) {}
    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    void reset(const QVector<AutoEqEntry> *entries, const QVector<int> &rows, const QSet<QString> *favorites);
    void favoriteChanged(const QString &id);
private:
    const QVector<AutoEqEntry> *m_entries = nullptr;
    const QSet<QString> *m_favorites = nullptr;
    QVector<int> m_rows;
};

// AutoEQ headphone profiles, as on macOS: the database bundled with Console
// (or the user's updated copy), search, favourites, applying a profile to
// inputs 1 and 2, and updating the database (import a file, rebuild from the
// AutoEQ project on GitHub, or go back to the bundled one). Updating is the
// core's (dspi_autoeq_*); this class supplies its HTTP GET and shows the result.
class AutoEqLibrary : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QObject *results READ results CONSTANT)
    Q_PROPERTY(QString query READ query WRITE setQuery NOTIFY queryChanged)
    Q_PROPERTY(int entryCount READ entryCount NOTIFY databaseChanged)
    Q_PROPERTY(QString databaseDate READ databaseDate NOTIFY databaseChanged)
    Q_PROPERTY(bool hasUserDatabase READ hasUserDatabase NOTIFY databaseChanged)
    Q_PROPERTY(QString loadError READ loadError NOTIFY databaseChanged)
    // [{id, name}] in the order they were added
    Q_PROPERTY(QVariantList favorites READ favorites NOTIFY favoritesChanged)
    Q_PROPERTY(bool rebuilding READ rebuilding NOTIFY rebuildChanged)
    Q_PROPERTY(double rebuildProgress READ rebuildProgress NOTIFY rebuildChanged)
    Q_PROPERTY(QString rebuildStatus READ rebuildStatus NOTIFY rebuildChanged)

public:
    explicit AutoEqLibrary(DSPiBridge *bridge, QObject *parent = nullptr);
    ~AutoEqLibrary() override;

    // The core's fetch: blocks its thread for one GET. Runs on the rebuild's
    // threads, up to 20 at once. `accept` is empty when no such header is wanted;
    // once `stop` is set the answer is no longer wanted.
    using Fetcher = std::function<AutoEqFetchAnswer(const QByteArray &url, const QByteArray &accept,
                                                    const QByteArray &userAgent, const std::atomic_bool &stop)>;
    // The real one: a QNetworkAccessManager and an event loop of its own on the
    // calling thread, which never waits on the GUI thread.
    static AutoEqFetchAnswer httpGet(const QByteArray &url, const QByteArray &accept,
                                     const QByteArray &userAgent, const std::atomic_bool &stop);
#ifdef DSPI_SMOKE
    void setFetcherForTest(const Fetcher &fetcher) { m_fetcher = fetcher; }
    QString userPathForTest() const { return userPath(); }
#endif

    QObject *results() { return &m_model; }
    QString query() const { return m_query; }
    void setQuery(const QString &q);
    int entryCount() const { return m_entries.size(); }
    QString databaseDate() const { return m_date; }
    bool hasUserDatabase() const;
    QString loadError() const { return m_error; }
    QVariantList favorites() const { return m_favoriteList; }
    bool rebuilding() const { return m_rebuilding; }
    double rebuildProgress() const { return m_progress; }
    QString rebuildStatus() const { return m_status; }

    // Read the database now (the browser does on opening)
    Q_INVOKABLE void load();
    // {id, name, source, formFactor} for one profile ({} if unknown)
    Q_INVOKABLE QVariantMap entry(const QString &id);
    Q_INVOKABLE void toggleFavorite(const QString &id);
    Q_INVOKABLE void clearFavorites();
    // Preamp and filters onto inputs 1 and 2; false without a device
    Q_INVOKABLE bool apply(const QString &id);

    // These two change the file off the GUI thread and end in updateFinished
    Q_INVOKABLE void importFile(const QUrl &file);
    Q_INVOKABLE void resetToBuiltIn();
    // What a rebuild reaches for and writes, to show before starting one
    Q_INVOKABLE QString rebuildWarning() const;
    Q_INVOKABLE void startRebuild();
    Q_INVOKABLE void cancelRebuild();

signals:
    void queryChanged();
    void databaseChanged();
    void favoritesChanged();
    void rebuildChanged();
    void rebuildFinished(bool ok, const QString &message);
    void updateFinished(bool ok, const QString &failedTitle, const QString &message);

private:
    bool readDatabase(const QByteArray &json, QString *error);
    void filter();
    QString userPath() const;
    void setRebuild(bool on, double progress, const QString &status);
    void pollRebuild();
    // Cancel the rebuild in hand and free it off the GUI thread: freeing waits
    // for the requests in flight
    void retireRebuild();
    void runOffGui(const std::function<void()> &work, const std::function<void()> &done);
    bool updateRunning() const { return m_rebuilding || m_updating; }

    DSPiBridge *m_bridge;
    AutoEqModel m_model;
    QVector<AutoEqEntry> m_entries;
    QHash<QString, int> m_byId;
    bool m_loaded = false;
    QString m_query;
    QString m_date;
    QString m_error;
    QSet<QString> m_favorites;
    QVariantList m_favoriteList;

    // Updating
    Fetcher m_fetcher;
    AutoEqRebuildHandle *m_handle = nullptr;
    std::shared_ptr<AutoEqFetchContext> m_fetchContext;
    QTimer m_poll;
    QList<QThread *> m_workers;
    bool m_rebuilding = false;
    bool m_updating = false;
    double m_progress = 0;
    QString m_status;
};

#endif // AUTOEQLIBRARY_H
