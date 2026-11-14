#ifndef AUTOEQCONTROLLER_H
#define AUTOEQCONTROLLER_H

#include <QAbstractListModel>
#include <QByteArray>
#include <QHash>
#include <QJsonObject>
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
    QString id;
    QString manufacturer, model;
    QString source;       // "oratory1990", "crinacle", ...
    QString formFactor;   // "over-ear", "in-ear", "earbud"
    float preamp = 0;
    QVector<Band> bands;
    QByteArray entry;   // -- WP P1QT2 -- the database's entry as JSON, for the core's preview and apply
    QString displayName() const;
};

// The browser's list: the profiles matching the search, in the database's order
class AutoEqProfileModel : public QAbstractListModel {
    Q_OBJECT
    QML_ANONYMOUS
    // What the person typed: the manufacturer, model or whole name holds it
    Q_PROPERTY(QString searchText READ searchText WRITE setSearchText NOTIFY searchTextChanged)
    Q_PROPERTY(int shown READ rowCount NOTIFY shownChanged)
public:
    enum Role { ProfileId = Qt::UserRole + 1, DisplayName, Source, SourceName, FormFactor, Favorite };
    explicit AutoEqProfileModel(QObject *parent = nullptr) : QAbstractListModel(parent) {}

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    QString searchText() const { return m_search; }
    void setSearchText(const QString &text);
    // A profile's row in the current list, or -1
    Q_INVOKABLE int rowOf(const QString &id) const;
    Q_INVOKABLE QString idAt(int row) const;

    void setProfiles(std::shared_ptr<const QVector<AutoEqProfile>> profiles);
    void setFavourites(const QStringList &ids);

signals:
    void searchTextChanged();
    void shownChanged();

private:
    void refilter();

    std::shared_ptr<const QVector<AutoEqProfile>> m_profiles;
    QVector<int> m_rows;   // indices into m_profiles
    QStringList m_favourites;
    QString m_search;
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
    Q_PROPERTY(bool loading READ loading NOTIFY libraryChanged)
    Q_PROPERTY(int profileCount READ profileCount NOTIFY libraryChanged)
    Q_PROPERTY(QString generatedOn READ generatedOn NOTIFY libraryChanged)
    Q_PROPERTY(QString loadProblem READ loadProblem NOTIFY libraryChanged)
    Q_PROPERTY(bool updatedDatabase READ updatedDatabase NOTIFY libraryChanged)
    // [{id, title}] in the order they were added (the menu's rows)
    Q_PROPERTY(QVariantList favouriteList READ favouriteList NOTIFY favouritesChanged)
    // A profile can be sent now: a device that takes writes
    Q_PROPERTY(bool canApply READ canApply NOTIFY canApplyChanged)
    // Updating: a file import or reset is running
    Q_PROPERTY(bool fileBusy READ fileBusy NOTIFY fileBusyChanged)
    // The rebuild's sheet: shown, fraction 0..1, the core's status line, done
    Q_PROPERTY(bool rebuildOnScreen READ rebuildOnScreen NOTIFY rebuildMoved)
    Q_PROPERTY(double rebuildFraction READ rebuildFraction NOTIFY rebuildMoved)
    Q_PROPERTY(QString rebuildLine READ rebuildLine NOTIFY rebuildMoved)
    Q_PROPERTY(bool rebuildFinished READ rebuildFinished NOTIFY rebuildMoved)
    // -- WP P1QT2 -- The channel pair a profile goes to (P1AQ: any pair):
    // [{ title, left, right, outputs }] by wire channel (inputs first, then
    // outputs), and the chosen one's index (inputs 1 and 2 first)
    Q_PROPERTY(QVariantList pairs READ pairs NOTIFY pairsChanged)
    Q_PROPERTY(int pair READ pair WRITE setPair NOTIFY pairsChanged)
    // -- WP P1QT2 -- What the chosen profile would do to the pair, before Apply
    // (dspi_autoeq_preview; nothing is sent): { profileId, curves: [{points,
    // color}], gridDb, gridY, levelText, notes } or {} with none
    Q_PROPERTY(QVariantMap preview READ preview NOTIFY previewChanged)
public:
    AutoEqController(const AppContext &ctx, QObject *parent = nullptr);
    ~AutoEqController() override;
    static AutoEqController *create(QQmlEngine *, QJSEngine *);

    AutoEqProfileModel *profiles() const { return m_model; }
    bool loading() const { return m_loading; }
    int profileCount() const { return m_profiles ? int(m_profiles->size()) : 0; }
    QString generatedOn() const { return m_date; }
    QString loadProblem() const { return m_error; }
    bool updatedDatabase() const;
    QVariantList favouriteList() const;
    bool canApply() const;
    bool fileBusy() const { return m_fileBusy; }
    bool rebuildOnScreen() const { return m_rebuild != nullptr || m_rebuildShown; }
    double rebuildFraction() const { return m_rebuildFraction; }
    QString rebuildLine() const { return m_rebuildLine; }
    bool rebuildFinished() const { return m_rebuildFinished; }

    // {name, sourceName, formFactor, formFactorIcon} of a profile, or {}
    Q_INVOKABLE QVariantMap profile(const QString &id) const;
    Q_INVOKABLE bool isFavourite(const QString &id) const;
    Q_INVOKABLE void flipFavourite(const QString &id);
    Q_INVOKABLE void forgetFavourites();
    // The profile's preamp and bands on the chosen pair, the rest of their
    // bands flat (-- WP P1QT2 -- the core's autoeq_apply: the state before
    // kept as a snapshot, one undo step, held to the limits profile)
    Q_INVOKABLE void apply(const QString &id);
    // -- WP P1QT2 --
    QVariantList pairs() const;
    int pair() const { return m_pair; }
    void setPair(int index);
    QVariantMap preview() const { return m_preview; }
    // Asks the core what `id` would do to the chosen pair ("" clears it)
    Q_INVOKABLE void requestPreview(const QString &id);

    // Update Database...: the dialog's text
    Q_INVOKABLE QString updateQuestionText() const;
    Q_INVOKABLE void importDatabaseFrom(const QString &path);
    Q_INVOKABLE void chooseDatabaseFile();
    Q_INVOKABLE void resetToBuiltIn();
    Q_INVOKABLE void rebuildFromGitHub();
    Q_INVOKABLE void stopRebuild();

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
    void libraryChanged();
    void favouritesChanged();
    void canApplyChanged();
    void fileBusyChanged();
    void rebuildMoved();
    void pairsChanged();     // -- WP P1QT2 --
    void previewChanged();   // -- WP P1QT2 --
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
    void setFileBusy(bool busy);
    void setFavouriteIds(const QStringList &ids);
    QString withCount(const QString &what) const;
    void pollRebuild();
    void endRebuild(bool succeeded, const QString &status);
    void freeRebuild();
    static QVariantMap previewView(const QJsonObject &reply);   // -- WP P1QT2 --

    AppContext m_ctx;
    AutoEqProfileModel *m_model;
    std::shared_ptr<const QVector<AutoEqProfile>> m_profiles;
    QHash<QString, int> m_byId;
    QStringList m_favouriteIds;
    bool m_loading = false;
    bool m_fileBusy = false;
    QString m_date, m_error;
    QThreadPool m_files;   // one thread: reading, importing, resetting the database
    AutoEqRebuildHandle *m_rebuild = nullptr;
    QByteArray m_rebuildPath;   // kept alive for the core while it runs
    QTimer m_rebuildPoll;
    bool m_rebuildShown = false;
    bool m_rebuildFinished = false;
    double m_rebuildFraction = 0;
    QString m_rebuildLine;
    bool m_lastCanApply = false;
    int m_pair = 0;              // -- WP P1QT2 --
    QVariantMap m_preview;       // -- WP P1QT2 --
    QString m_previewId;         // -- WP P1QT2 --
    quint64 m_previewSeq = 0;    // -- WP P1QT2 --
    QVariantList m_pairsShown;   // -- WP P1QT2 --
};

#endif // AUTOEQCONTROLLER_H
