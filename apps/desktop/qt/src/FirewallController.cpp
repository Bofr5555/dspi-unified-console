#include "FirewallController.h"

#include <QCoreApplication>
#include <QDesktopServices>
#include <QDir>
#include <QUrl>

#include "Registrar.h"

// -- WP UPW2 --
static const Registrar::Controller<FirewallController> firewallRegistered("firewall");

#ifdef Q_OS_WIN

#include <winsock2.h>
#include <windows.h>
#include <iphlpapi.h>
#include <netfw.h>
#include <netlistmgr.h>
#include <oleauto.h>
#include <shellapi.h>

namespace {

// Spelled out, as the class ids need a GUID library to link
const CLSID kNetFwPolicy2 = { 0xe2b3c97f, 0x6ae1, 0x41ac, { 0x81, 0x7a, 0xf6, 0xf9, 0x21, 0x66, 0xd7, 0xdd } };
const CLSID kNetworkListManager = { 0xdcb00c01, 0x570f, 0x4a9b, { 0x8d, 0x69, 0x19, 0x9f, 0xdb, 0xa5, 0x72, 0x3b } };

// The rule's name in Windows Defender Firewall's list
const wchar_t *const kRuleName = L"DSPi uConsole";

template <class T>
struct Com {
    T *p = nullptr;
    Com() = default;
    Com(const Com &) = delete;
    Com &operator=(const Com &) = delete;
    ~Com() { if (p) p->Release(); }
    T *operator->() const { return p; }
};

struct Bstr {
    BSTR s = nullptr;
    Bstr() = default;
    Bstr(const Bstr &) = delete;
    Bstr &operator=(const Bstr &) = delete;
    ~Bstr() { SysFreeString(s); }
};

struct Variant {
    VARIANT v;
    Variant() { VariantInit(&v); }
    Variant(const Variant &) = delete;
    Variant &operator=(const Variant &) = delete;
    ~Variant() { VariantClear(&v); }
};

// A rule's program, as Windows stores it (it may hold %ProgramFiles% and the like)
QString expanded(const BSTR path) {
    if (!path) return {};
    wchar_t out[MAX_PATH * 2];
    const DWORD n = ExpandEnvironmentStringsW(path, out, DWORD(sizeof out / sizeof *out));
    if (n == 0 || n > sizeof out / sizeof *out) return QString::fromWCharArray(path);
    return QString::fromWCharArray(out);
}

// Inbound rules for `exe` that are on for any of `profiles`: a Block rule wins over
// any Allow rule, as in Windows Firewall itself
QString readRules(const QString &exe) {
    Com<INetFwPolicy2> policy;
    if (FAILED(CoCreateInstance(kNetFwPolicy2, nullptr, CLSCTX_INPROC_SERVER, __uuidof(INetFwPolicy2),
                                reinterpret_cast<void **>(&policy.p))))
        return {};
    // Judged for the networks other Consoles are meant to reach this one on. A
    // Public network is reported apart: the fix is to make it Private.
    long current = 0;
    if (FAILED(policy->get_CurrentProfileTypes(&current))) return {};
    long profiles = current & (NET_FW_PROFILE2_PRIVATE | NET_FW_PROFILE2_DOMAIN);
    if (profiles == 0) profiles = NET_FW_PROFILE2_PRIVATE;
    long on = 0;
    for (const long p : { long(NET_FW_PROFILE2_PRIVATE), long(NET_FW_PROFILE2_DOMAIN) }) {
        VARIANT_BOOL enabled = VARIANT_FALSE;
        if ((profiles & p) && SUCCEEDED(policy->get_FirewallEnabled(NET_FW_PROFILE_TYPE2(p), &enabled))
            && enabled == VARIANT_TRUE)
            on |= p;
    }
    if (on == 0) return QStringLiteral("off");

    Com<INetFwRules> rules;
    if (FAILED(policy->get_Rules(&rules.p))) return {};
    Com<IUnknown> unknown;
    if (FAILED(rules->get__NewEnum(&unknown.p))) return {};
    Com<IEnumVARIANT> items;
    if (FAILED(unknown->QueryInterface(__uuidof(IEnumVARIANT), reinterpret_cast<void **>(&items.p)))) return {};
    bool allowed = false, blocked = false;
    for (;;) {
        Variant item;
        ULONG fetched = 0;
        if (items->Next(1, &item.v, &fetched) != S_OK || fetched == 0) break;
        if (item.v.vt != VT_DISPATCH || !item.v.pdispVal) continue;
        Com<INetFwRule> rule;
        if (FAILED(item.v.pdispVal->QueryInterface(__uuidof(INetFwRule), reinterpret_cast<void **>(&rule.p))))
            continue;
        VARIANT_BOOL enabled = VARIANT_FALSE;
        NET_FW_RULE_DIRECTION direction = NET_FW_RULE_DIR_OUT;
        long ruleProfiles = 0;
        if (FAILED(rule->get_Enabled(&enabled)) || enabled != VARIANT_TRUE) continue;
        if (FAILED(rule->get_Direction(&direction)) || direction != NET_FW_RULE_DIR_IN) continue;
        if (FAILED(rule->get_Profiles(&ruleProfiles)) || (ruleProfiles & on) == 0) continue;
        Bstr app;
        if (FAILED(rule->get_ApplicationName(&app.s)) || !app.s) continue;
        if (QDir::toNativeSeparators(expanded(app.s)).compare(exe, Qt::CaseInsensitive) != 0) continue;
        NET_FW_ACTION action = NET_FW_ACTION_BLOCK;
        if (FAILED(rule->get_Action(&action))) continue;
        if (action == NET_FW_ACTION_BLOCK) blocked = true;
        else if (action == NET_FW_ACTION_ALLOW) allowed = true;
    }
    return blocked ? QStringLiteral("blocked") : allowed ? QStringLiteral("allowed") : QStringLiteral("none");
}

// The adapters that are up and have a default gateway: the ones other Consoles
// are on. The hub listens and advertises on every adapter, so this is what leaves
// out host-only virtual ones (Hyper-V, WSL, VirtualBox), which Windows files as an
// "Unidentified network", Public, and nobody reaches the hub through.
// Limit: a LAN with no router (a direct cable) is left out too.
QList<GUID> gatewayAdapters() {
    ULONG size = 16 * 1024;
    QByteArray buf;
    ULONG result = ERROR_BUFFER_OVERFLOW;
    for (int tries = 0; tries < 3 && result == ERROR_BUFFER_OVERFLOW; ++tries) {
        buf.resize(int(size));
        result = GetAdaptersAddresses(AF_UNSPEC,
                                      GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST
                                          | GAA_FLAG_SKIP_DNS_SERVER,
                                      nullptr, reinterpret_cast<IP_ADAPTER_ADDRESSES *>(buf.data()), &size);
    }
    QList<GUID> out;
    if (result != NO_ERROR) return out;
    for (auto *a = reinterpret_cast<IP_ADAPTER_ADDRESSES *>(buf.data()); a; a = a->Next) {
        if (a->OperStatus != IfOperStatusUp || !a->FirstGatewayAddress || !a->AdapterName) continue;
        // AdapterName is the adapter's GUID, as "{…}"
        const std::wstring name = QString::fromLatin1(a->AdapterName).toStdWString();
        GUID id;
        if (SUCCEEDED(CLSIDFromString(name.c_str(), &id))) out.append(id);
    }
    return out;
}

// Whether a connected Public network is on one of gatewayAdapters()
bool onPublicNetwork() {
    const QList<GUID> adapters = gatewayAdapters();
    Com<INetworkListManager> manager;
    if (FAILED(CoCreateInstance(kNetworkListManager, nullptr, CLSCTX_ALL, __uuidof(INetworkListManager),
                                reinterpret_cast<void **>(&manager.p))))
        return false;
    Com<IEnumNetworks> networks;
    if (FAILED(manager->GetNetworks(NLM_ENUM_NETWORK_CONNECTED, &networks.p)) || !networks.p) return false;
    for (;;) {
        Com<INetwork> network;
        ULONG fetched = 0;
        if (networks->Next(1, &network.p, &fetched) != S_OK || fetched == 0) break;
        NLM_NETWORK_CATEGORY category = NLM_NETWORK_CATEGORY_PRIVATE;
        if (FAILED(network->GetCategory(&category)) || category != NLM_NETWORK_CATEGORY_PUBLIC) continue;
        Com<IEnumNetworkConnections> connections;
        if (FAILED(network->GetNetworkConnections(&connections.p)) || !connections.p) continue;
        for (;;) {
            Com<INetworkConnection> connection;
            ULONG got = 0;
            if (connections->Next(1, &connection.p, &got) != S_OK || got == 0) break;
            GUID adapter;
            if (SUCCEEDED(connection->GetAdapterId(&adapter)) && adapters.contains(adapter)) return true;
        }
    }
    return false;
}

// netsh itself, elevated, by its full path in the system directory: no command
// interpreter in between and no PATH or current-directory search. One command
// per case, so one UAC prompt: Block rules (a Cancel left them, and they win
// over any Allow rule) are turned into Allow rules, else one Allow rule is
// added. netsh splits its own arguments and expands nothing, so the quoted path
// is safe whatever it holds: a path cannot contain `"`.
// Returns "" when the rule is in place, else why not.
QString addAllowRule(const QString &exe, bool blocked) {
    // Windows Firewall expands environment variables in a rule's program, so a
    // path with % in it cannot be named in a rule at all (netsh refuses it, or
    // the rule names another path).
    if (exe.contains(QLatin1Char('%')))
        return FirewallController::tr("Windows Firewall cannot name a program whose folder has a % sign in its name. Move DSPi uConsole to another folder.");
    wchar_t system[MAX_PATH];
    const UINT n = GetSystemDirectoryW(system, MAX_PATH);
    if (n == 0 || n >= MAX_PATH) return FirewallController::tr("Windows could not start the change (error %1).").arg(GetLastError());
    const std::wstring directory(system, n);
    const std::wstring netsh = directory + L"\\netsh.exe";
    const QString program = QStringLiteral("dir=in program=\"%1\"").arg(exe);
    const QString command = blocked
        ? QStringLiteral("advfirewall firewall set rule name=all %1 new action=allow enable=yes profile=private,domain").arg(program)
        : QStringLiteral("advfirewall firewall add rule name=\"%1\" %2 action=allow enable=yes profile=private,domain")
              .arg(QString::fromWCharArray(kRuleName), program);
    const std::wstring params = command.toStdWString();
    SHELLEXECUTEINFOW info = {};
    info.cbSize = sizeof info;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = L"runas";
    info.lpFile = netsh.c_str();
    info.lpParameters = params.c_str();
    info.lpDirectory = directory.c_str();
    info.nShow = SW_HIDE;
    if (!ShellExecuteExW(&info)) {
        if (GetLastError() == ERROR_CANCELLED) return FirewallController::tr("Windows did not get permission, so nothing changed.");
        return FirewallController::tr("Windows could not start the change (error %1).").arg(GetLastError());
    }
    if (!info.hProcess) return {};
    DWORD code = 1;
    if (WaitForSingleObject(info.hProcess, 60000) != WAIT_OBJECT_0) {
        CloseHandle(info.hProcess);
        return FirewallController::tr("Windows Firewall did not answer in time.");
    }
    GetExitCodeProcess(info.hProcess, &code);
    CloseHandle(info.hProcess);
    return code == 0 ? QString() : FirewallController::tr("Windows Firewall refused the rule (code %1).").arg(code);
}

// A worker thread of ours: it joins the multithreaded apartment for the call
template <class F>
auto withCom(F f) -> decltype(f()) {
    const HRESULT init = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    auto out = f();
    if (SUCCEEDED(init)) CoUninitialize();
    return out;
}

} // namespace

#endif // Q_OS_WIN

FirewallController::FirewallController(DSPiBridge *, QObject *parent)
    : QObject(parent), m_exe(QDir::toNativeSeparators(QCoreApplication::applicationFilePath()))
{
    m_worker.setMaxThreadCount(1);
}

FirewallController::~FirewallController() { m_worker.waitForDone(); }

bool FirewallController::supported() const
{
#ifdef Q_OS_WIN
    return true;
#else
    return false;
#endif
}

void FirewallController::check()
{
#ifdef Q_OS_WIN
    if (m_checking) return;
    m_checking = true;
    emit changed();
    const QString exe = m_exe;
    m_worker.start([this, exe]() {
        const QString rules = withCom([&] { return readRules(exe); });
        const bool isPublic = withCom([] { return onPublicNetwork(); });
        QMetaObject::invokeMethod(this, [this, rules, isPublic]() {
            m_checking = false;
            m_rules = rules;
            m_public = isPublic;
            emit changed();
        }, Qt::QueuedConnection);
    });
#endif
}

void FirewallController::allow()
{
#ifdef Q_OS_WIN
    if (m_fixing) return;
    m_fixing = true;
    m_fixError.clear();
    emit changed();
    const QString exe = m_exe;
    const bool blocked = m_rules == QLatin1String("blocked");
    m_worker.start([this, exe, blocked]() {
        const QString error = withCom([&] { return addAllowRule(exe, blocked); });
        QMetaObject::invokeMethod(this, [this, error]() {
            m_fixing = false;
            m_fixError = error;
            emit changed();
            check();
        }, Qt::QueuedConnection);
    });
#endif
}

void FirewallController::openNetworkSettings()
{
    // Settings > Network & internet, where a network's profile type is chosen
    QDesktopServices::openUrl(QUrl(QStringLiteral("ms-settings:network-status")));
}
