#ifndef AUTOEQCONTROLLER_H
#define AUTOEQCONTROLLER_H

#include <QAbstractListModel>
#include <QByteArray>
#include <QHash>
#include <QObject>
#include <QStringList>
#include <QThreadPool>
#include <QTimer>
#include <QVariantList>
#include <QVector>
#include <QtQml/qqmlregistration.h>
#include <atomic>
#include <functional>
#include <memory>
#include <mutex>

#include "CoreApi.h"
#include "Registrar.h"

class QJSEngine;
class QQmlEngine;

// One headphone's profile from the database (AutoEQManager.swift's
// HeadphoneEntry): its preamp and up to ten bands
struct AutoEqProfile {
    struct Band {
        uint8_t type;   // FILTER_*
        float freq, q, gain;
    };
    QString id, manufacturer, model, source, formFactor;
    float preamp = 0;
    QVector<Band> bands;
    QString displayName() const;
};

// The browser's list: the profiles matching the search, in the database's order
class AutoEqProfileModel : public QAbstractListModel {
    Q_OBJECT
    QML_ANONYMOUS
    Q_PROPERTY(QString query READ query WRITE setQuery NOTIFY queryChanged)
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)
public:
    enum Role { ProfileId = Qt::UserRole + 1, DisplayName, Source, SourceName, FormFactor, Favorite };
    explicit AutoEqProfileModel(QObject *parent = nullptr) : QAbstractListModel(parent) {}

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    QString query() const { return m_query; }
    void setQuery(const QString &query);
    // A profile's row in the current list, or -1
    Q_INVOKABLE int rowOf(const QString &id) const;
    Q_INVOKABLE QString idAt(int row) const;

    void setProfiles(std::shared_ptr<const QVector<AutoEqProfile>> profiles);
    void setFavorites(const QStringList &ids);

signals:
    void queryChanged();
    void countChanged();

private:
    void refilter();

    std::shared_ptr<const QVector<AutoEqProfile>> m_profiles;
    QVector<int> m_rows;   // indices into m_profiles
    QStringList m_favorites;
    QString m_query;
};

// AutoEQ (AutoEQManager.swift, AutoEQBrowser.swift and the AutoEQ menu): the
// headphone database, the browser's search, favourites, applying a profile to
// inputs 1 and 2, and keeping the database up to date: rebuild from GitHub,
// import a file, reset to the built-in copy. QML sees the singleton
// `AutoEqController`.
//
// The built-in database is the one the other clients ship; an updated one is
// the user's file in the app's data folder. The core checks, imports, resets
// and rebuilds it (dspi_autoeq_*); this reads it for the list, off the GUI
// thread, and answers the rebuild's requests with Qt's network access.
class AutoEqController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(AutoEqProfileModel *profiles READ profiles CONSTANT)
    Q_PROPERTY(bool loading READ loading NOTIFY databaseChanged)
    Q_PROPERTY(int entryCount READ entryCount NOTIFY databaseChanged)
    Q_PROPERTY(QString databaseDate READ databaseDate NOTIFY databaseChanged)
    Q_PROPERTY(QString errorMessage READ errorMessage NOTIFY databaseChanged)
    Q_PROPERTY(bool hasUserDatabase READ hasUserDatabase NOTIFY databaseChanged)
    // [{id, title}] in the order they were added (the menu's rows)
    Q_PROPERTY(QVariantList favorites READ favorites NOTIFY favoritesChanged)
    // A profile can be sent now: a device that takes writes
    Q_PROPERTY(bool canApply READ canApply NOTIFY canApplyChanged)
    // Updating: a file import or reset is running
    Q_PROPERTY(bool updating READ updating NOTIFY updatingChanged)
    // The rebuild's sheet: shown, fraction 0..1, the core's status line, done
    Q_PROPERTY(bool rebuilding READ rebuilding NOTIFY rebuildChanged)
    Q_PROPERTY(double rebuildProgress READ rebuildProgress NOTIFY rebuildChanged)
    Q_PROPERTY(QString rebuildStatus READ rebuildStatus NOTIFY rebuildChanged)
    Q_PROPERTY(bool rebuildDone READ rebuildDone NOTIFY rebuildChanged)
public:
    AutoEqController(const AppContext &ctx, QObject *parent = nullptr);
    ~AutoEqController() override;
    static AutoEqController *create(QQmlEngine *, QJSEngine *);

    AutoEqProfileModel *profiles() const { return m_model; }
    bool loading() const { return m_loading; }
    int entryCount() const { return m_profiles ? int(m_profiles->size()) : 0; }
    QString databaseDate() const { return m_date; }
    QString errorMessage() const { return m_error; }
    bool hasUserDatabase() const;
    QVariantList favorites() const;
    bool canApply() const;
    bool updating() const { return m_updating; }
    bool rebuilding() const { return m_rebuild != nullptr || m_rebuildShown; }
    double rebuildProgress() const { return m_rebuildProgress; }
    QString rebuildStatus() const { return m_rebuildStatus; }
    bool rebuildDone() const { return m_rebuildDone; }

    // {name, sourceName, formFactor, formFactorIcon} of a profile, or {}
    Q_INVOKABLE QVariantMap profile(const QString &id) const;
    Q_INVOKABLE bool isFavorite(const QString &id) const;
    Q_INVOKABLE void toggleFavorite(const QString &id);
    Q_INVOKABLE void clearFavorites();
    // The profile's preamp and bands on inputs 1 and 2, the rest of their bands flat
    Q_INVOKABLE void apply(const QString &id);

    // Update Database...: the dialog's text
    Q_INVOKABLE QString updateText() const;
    Q_INVOKABLE void importDatabaseFrom(const QString &path);
    Q_INVOKABLE void chooseDatabaseFile();
    Q_INVOKABLE void resetToBuiltIn();
    Q_INVOKABLE void startRebuild();
    Q_INVOKABLE void cancelRebuild();

    // The user's database file, and the built-in one (a resource)
    static QString userDatabasePath();
    static QString builtInDatabasePath();

    // The rebuild's HTTP GET: blocks until answered (own manager and event
    // loop on the calling thread). Any thread.
    struct Answer {
        bool answered = false;
        int status = 0;
        QByteArray body;
        qint64 retryAfter = -1, remaining = -1, reset = -1;
        QString error;   // why no answer came
    };
    static Answer fetch(const QByteArray &url, const QByteArray &accept, const QByteArray &userAgent);
    // Stands in for fetch() while set (the smoke driver's scripted GitHub). Set
    // it before a rebuild starts.
    using FetchOverride = std::function<Answer(const QByteArray &url, const QByteArray &accept, const QByteArray &agent)>;
    static void setFetchOverride(FetchOverride fetch);

signals:
    void databaseChanged();
    void favoritesChanged();
    void canApplyChanged();
    void updatingChanged();
    void rebuildChanged();
    // The Update Database dialog: the host shows it
    void updateRequested();
    // An alert after an update: `error` gives it the error look
    void notice(const QString &title, const QString &text, bool error);

private:
    struct Loaded {
        std::shared_ptr<QVector<AutoEqProfile>> profiles;
        QString date, error;
    };
    static Loaded load(const QString &path);
    static void fetchThunk(void *self, const char *url, const char *accept, const char *agent, AutoEqFetchSink *sink);
    void reload(std::function<void()> then = {});
    void setUpdating(bool updating);
    void setFavoriteIds(const QStringList &ids);
    void pollRebuild();
    void endRebuild(bool succeeded, const QString &status);
    void freeRebuild();

    AppContext m_ctx;
    AutoEqProfileModel *m_model;
    std::shared_ptr<const QVector<AutoEqProfile>> m_profiles;
    QHash<QString, int> m_byId;
    QStringList m_favoriteIds;
    bool m_loading = false;
    bool m_updating = false;
    QString m_date, m_error;
    QThreadPool m_files;   // one thread: reading, importing, resetting the database
    AutoEqRebuildHandle *m_rebuild = nullptr;
    QByteArray m_rebuildPath;   // kept alive for the core while it runs
    QTimer m_rebuildPoll;
    bool m_rebuildShown = false;
    bool m_rebuildDone = false;
    double m_rebuildProgress = 0;
    QString m_rebuildStatus;
    bool m_lastCanApply = false;
};

#endif // AUTOEQCONTROLLER_H
