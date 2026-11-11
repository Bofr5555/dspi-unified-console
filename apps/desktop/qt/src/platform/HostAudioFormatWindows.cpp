#include "HostAudioFormat.h"

#ifdef Q_OS_WIN

#include <QLoggingCategory>
#include <QStringList>
#include <atomic>
#include <string>
#include <windows.h>
#include <cfgmgr32.h>
#include <mmdeviceapi.h>
#include <propidl.h>
#include <propsys.h>

namespace HostAudio {

namespace {
// Spelled out, as FunctionDiscoveryKeys need a GUID library to link
const PROPERTYKEY kFriendlyName = { { 0xa45c254e, 0xdf1c, 0x4efd, { 0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0 } }, 14 };
// The adapter's name, which survives the user renaming the endpoint
const PROPERTYKEY kInterfaceName = { { 0x026e516e, 0xb814, 0x414b, { 0x83, 0xcd, 0x85, 0x6d, 0x6f, 0xef, 0x48, 0x22 } }, 2 };
// The format picked under Sound settings, kept while nothing plays
const PROPERTYKEY kDeviceFormat = { { 0xf19f064d, 0x082c, 0x4e27, { 0xbc, 0x73, 0x68, 0x82, 0xa1, 0xbb, 0x8e, 0x4c } }, 0 };

template <class T>
struct Com {
    T *p = nullptr;
    Com() = default;
    Com(const Com &) = delete;
    Com &operator=(const Com &) = delete;
    ~Com() { if (p) p->Release(); }
    T *operator->() const { return p; }
};

// -- WP UPW2 -- QT_LOGGING_RULES="dspi.hostaudio=true" lists each endpoint's chain
Q_LOGGING_CATEGORY(lcHostAudio, "dspi.hostaudio", QtWarningMsg)

// The endpoint's device node (SWD\MMDEVAPI\<its id>), then its parents up to the
// root: the audio function, then the USB device that holds the serial
QStringList instanceChain(IMMDevice *device) {
    QStringList chain;
    LPWSTR id = nullptr;
    if (FAILED(device->GetId(&id)) || !id) return chain;
    std::wstring node = L"SWD\\MMDEVAPI\\";
    node += id;
    CoTaskMemFree(id);
    DEVINST inst = 0;
    if (CM_Locate_DevNodeW(&inst, const_cast<DEVINSTID_W>(node.c_str()), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS)
        return chain;
    // A few levels are all there is; the bound keeps a loop in a broken tree short
    for (int depth = 0; depth < 8; depth++) {
        wchar_t text[MAX_DEVICE_ID_LEN + 1] = {};
        if (CM_Get_Device_IDW(inst, text, MAX_DEVICE_ID_LEN, 0) != CR_SUCCESS) break;
        chain << QString::fromWCharArray(text);
        DEVINST parent = 0;
        if (CM_Get_Parent(&parent, inst, 0) != CR_SUCCESS) break;
        inst = parent;
    }
    return chain;
}

struct Prop {
    PROPVARIANT value;
    Prop() { PropVariantInit(&value); }
    Prop(const Prop &) = delete;
    Prop &operator=(const Prop &) = delete;
    ~Prop() { PropVariantClear(&value); }
};

QString stringProp(IPropertyStore *store, const PROPERTYKEY &key) {
    Prop prop;
    if (FAILED(store->GetValue(key, &prop.value)) || prop.value.vt != VT_LPWSTR || !prop.value.pwszVal) return {};
    return QString::fromWCharArray(prop.value.pwszVal);
}

int formatChannels(IPropertyStore *store) {
    Prop prop;
    if (FAILED(store->GetValue(kDeviceFormat, &prop.value)) || prop.value.vt != VT_BLOB) return 0;
    return waveFormatChannels(reinterpret_cast<const char *>(prop.value.blob.pBlobData), qint64(prop.value.blob.cbSize));
}

// Called on COM's threads. It only asks for a read: a callback must not block
class Notifications : public IMMNotificationClient {
public:
    explicit Notifications(std::function<void()> changed) : m_changed(std::move(changed)) {}

    ULONG STDMETHODCALLTYPE AddRef() override { return ++m_refs; }
    ULONG STDMETHODCALLTYPE Release() override {
        const ULONG left = --m_refs;
        if (left == 0) delete this;
        return left;
    }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void **out) override {
        if (!out) return E_POINTER;
        if (iid == __uuidof(IUnknown) || iid == __uuidof(IMMNotificationClient)) {
            *out = static_cast<IMMNotificationClient *>(this);
            AddRef();
            return S_OK;
        }
        *out = nullptr;
        return E_NOINTERFACE;
    }
    HRESULT STDMETHODCALLTYPE OnDeviceStateChanged(LPCWSTR, DWORD) override { m_changed(); return S_OK; }
    HRESULT STDMETHODCALLTYPE OnDeviceAdded(LPCWSTR) override { m_changed(); return S_OK; }
    HRESULT STDMETHODCALLTYPE OnDeviceRemoved(LPCWSTR) override { m_changed(); return S_OK; }
    HRESULT STDMETHODCALLTYPE OnDefaultDeviceChanged(EDataFlow, ERole, LPCWSTR) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE OnPropertyValueChanged(LPCWSTR, const PROPERTYKEY key) override {
        if (key.fmtid == kDeviceFormat.fmtid && key.pid == kDeviceFormat.pid) m_changed();
        return S_OK;
    }

private:
    virtual ~Notifications() = default;
    std::function<void()> m_changed;
    std::atomic<ULONG> m_refs { 1 };
};

class WasapiBackend : public Backend {
public:
    ~WasapiBackend() override { WasapiBackend::stopWatching(); }

    std::vector<Endpoint> read() override {
        // A worker thread of ours: it joins the multithreaded apartment for this read
        const HRESULT init = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
        std::vector<Endpoint> out = readEndpoints();
        if (SUCCEEDED(init)) CoUninitialize();
        return out;
    }

    // On the UI thread, where Qt has COM up already. Making the enumerator is in
    // process and quick, and keeps it in an apartment that outlives it; the
    // callbacks come on COM's threads either way.
    void startWatching(std::function<void()> changed) override {
        if (FAILED(CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL, __uuidof(IMMDeviceEnumerator),
                                    reinterpret_cast<void **>(&m_enumerator))))
            return;
        m_notifications = new Notifications(std::move(changed));
        if (FAILED(m_enumerator->RegisterEndpointNotificationCallback(m_notifications))) {
            m_notifications->Release();
            m_notifications = nullptr;
        }
    }

    void stopWatching() override {
        if (m_notifications) {
            m_enumerator->UnregisterEndpointNotificationCallback(m_notifications);
            m_notifications->Release();
            m_notifications = nullptr;
        }
        if (m_enumerator) {
            m_enumerator->Release();
            m_enumerator = nullptr;
        }
    }

private:
    static std::vector<Endpoint> readEndpoints() {
        std::vector<Endpoint> out;
        Com<IMMDeviceEnumerator> enumerator;
        if (FAILED(CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL, __uuidof(IMMDeviceEnumerator),
                                    reinterpret_cast<void **>(&enumerator.p))))
            return out;
        Com<IMMDeviceCollection> devices;
        if (FAILED(enumerator->EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE, &devices.p))) return out;
        UINT count = 0;
        if (FAILED(devices->GetCount(&count))) return out;
        for (UINT i = 0; i < count; i++) {
            Com<IMMDevice> device;
            if (FAILED(devices->Item(i, &device.p))) continue;
            Com<IPropertyStore> store;
            if (FAILED(device->OpenPropertyStore(STGM_READ, &store.p))) continue;
            Endpoint e;
            e.text = stringProp(store.p, kFriendlyName) + QLatin1Char(' ') + stringProp(store.p, kInterfaceName);
            e.channels = formatChannels(store.p);
            // -- WP UPW2 -- An endpoint's properties hold no serial, but the USB
            // device above it in the device tree does
            const QStringList chain = instanceChain(device.p);
            e.serialText = usbSerialFromInstanceIds(chain);
            qCDebug(lcHostAudio) << e.text << e.channels << "channels, serial" << e.serialText << chain;
            out.push_back(e);
        }
        return out;
    }

    IMMDeviceEnumerator *m_enumerator = nullptr;
    Notifications *m_notifications = nullptr;
};
}

std::unique_ptr<Backend> makeBackend() { return std::make_unique<WasapiBackend>(); }

} // namespace HostAudio

#endif // Q_OS_WIN
