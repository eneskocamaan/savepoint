<p align="center">
  <img src="assets/logo.svg" width="84" alt="Savepoint logosu">
</p>

<h1 align="center">Savepoint</h1>

<p align="center">
  Bilgisayar oyunlarının kayıtlarını otomatik ve sürümlü olarak yedekler.<br>
  Küçük, sessiz ve ücretsiz. <b>Windows 10 ve 11.</b>
</p>

<p align="center">
  <a href="https://github.com/eneskocamaan/savepoint/releases/latest"><img alt="Son sürüm" src="https://img.shields.io/github/v/release/eneskocamaan/savepoint?label=indir"></a>
  <a href="https://github.com/eneskocamaan/savepoint/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/eneskocamaan/savepoint/ci.yml?branch=main&label=CI"></a>
  <img alt="Windows 10 | 11" src="https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D6">
  <a href="LICENSE"><img alt="MIT Lisansı" src="https://img.shields.io/github/license/eneskocamaan/savepoint"></a>
</p>

<p align="center"><a href="README.md">English</a> · <a href="https://github.com/eneskocamaan/savepoint/releases/latest">İndir</a> · <a href="CONTRIBUTING.md">Katkıda bulun</a></p>

<p align="center">
  <img src="docs/screenshot.png" alt="Savepoint: oyun listesi, bir oyunun yedekleri ve ayarlar" width="900">
</p>

Savepoint bildirim alanında durur ve sen oynarken hiçbir şey yapmaz. Bir oyunu kapattığında o oyunun kayıt dosyalarına bakar, bir şey değiştiyse yeni bir kopya saklar. Bir kayıt bozulursa ya da önceki bir noktaya dönmek istersen tek tıkla geri yüklersin.

Yedek klasörü olarak Google Drive, OneDrive ya da Dropbox klasörünü seçersen kayıtların bulutta da olur ve başka bir bilgisayarda geri yüklenebilir.

## İndirme

**[Son sürüm](https://github.com/eneskocamaan/savepoint/releases/latest)** sayfasından:

| Dosya | Ne işe yarar |
|---|---|
| `Savepoint-<sürüm>-windows-x64-setup.exe` | **Önerilen.** Sadece senin Windows kullanıcın için kurar (yönetici izni gerekmez), Başlat menüsüne ekler, istersen Windows ile başlatır. Kaldırıcısı vardır. |
| `Savepoint-<sürüm>-windows-x64-portable.zip` | Kurulum yok. İstediğin yere aç, `savepointd.exe`'yi çalıştır. |
| Kaynak kod (zip / tar.gz) | GitHub her sürüme kendisi ekler. |

Hiçbir ayar gerekmez: ilk açılışta Savepoint oyun listesini indirir, oyunlarını bulur ve penceresini açar. Sen başka bir klasör seçene kadar yedekler `Belgeler\Savepoint Backups` klasörüne gider.

> **Windows SmartScreen** dosyalar henüz kod imzalı olmadığı için "Windows bilgisayarınızı korudu" diyebilir. *Ek bilgi → Yine de çalıştır*'a bas. İndirdiğin dosyanın yayınlanan dosya olduğunu, PowerShell'de `Get-FileHash <dosya>` sonucunu sürümdeki `SHA256SUMS.txt` ile karşılaştırarak doğrulayabilirsin.

Pencere Microsoft Edge WebView2 kullanır. Windows 11 ve güncel Windows 10 ile birlikte gelir, eksikse kurulum bunu söyler.

## Özellikler

- **Oyunları kendisi bulur.** 20.000'den fazla oyunun kayıt konumu topluluk tarafından tutulan [Ludusavi Manifest](https://github.com/mtkennerly/ludusavi-manifest)'ten gelir. Kurulu oyunlar Steam, Epic Games, GOG Galaxy, Xbox uygulaması, Windows'a kayıtlı programlar (kurulum paketleri ve repack'ler dahil) ve yaygın oyun klasörlerinden bulunur. Steam emülatörüyle çalışan kopyalar `steam_appid.txt` dosyalarından ve emülatörün kendi kayıt klasörlerinden tanınır.
- **Oyunu kapatınca yedekler.** Zamanlayıcı yok, oynarken tarama yok: Windows bir oyunun öne geldiğini bildirir, Savepoint kapanmasını bekler.
- **Sadece bir şey değiştiyse.** Önce boyut ve tarih karşılaştırılır, içerik (BLAKE3) sadece gerekirse. Değişmeyen dosyalar tekrar kopyalanmaz, önceki yedeğe hard link ile bağlanır.
- **Güvenli geri yükleme.** Hiçbir şeyin üzerine yazılmadan önce kaç dosyanın yazılacağı ve hangilerinin o yedekten sonra değiştiği gösterilir, mevcut kayıtlar da önce yedeklenir, yani her geri yükleme geri alınabilir.
- **Bozuk kayıt koruması.** Bir kayıt dosyası boş ya da öncekinden çok küçük çıkarsa (genelde bir çökme sonucu) yedek işaretlenir, uyarı gösterilir ve son sağlam yedek otomatik temizlikte asla silinmez.
- **Başka bilgisayara taşıma.** Dosyalar kullanıcı adına göre değil Belgeler ya da AppData gibi klasörlere göre saklanır, böylece yedek, kullanıcı adı ya da Steam kütüphanesi farklı bir bilgisayarda da geri yüklenebilir.
- **Bir şey ters giderse söyler.** Başarısız bir yedek (disk dolu, yedek klasörüne ulaşılamıyor, kayıt başka bir program tarafından kilitli) sessizce kaybolmaz, kırmızı bir şerit çıkar.
- **Hafif.** Boştayken yaklaşık 2 MB bellek, ölçülemeyecek kadar az işlemci (geliştiricinin bilgisayarında ölçüldü). Pencere ayrı bir programdır ve sadece açıkken çalışır.
- **İngilizce, Türkçe ve İspanyolca.** Yeni dil eklemek tek bir dosyayı çevirmek demek, bkz. [CONTRIBUTING.md](CONTRIBUTING.md#translations).

Tanımadığı oyunlar **Oyun ekle** ile elle eklenebilir (ad, kayıt klasörü ve isteğe bağlı olarak ikon ve otomatik yedek için oyunun exe dosyası).

## Gizlilik

- Hesap yok, telemetri yok, reklam yok.
- Tek ağ erişimi, oyun listesini haftada en fazla bir kez GitHub'dan (`raw.githubusercontent.com`) Windows'un kendi `curl.exe`'siyle indirmek.
- Ayarlar, oyun listesi önbelleği ve küçük bir günlük dosyası `%LOCALAPPDATA%\Savepoint` içinde durur. Yedekler sadece senin seçtiğin yere gider.

## Sorular

**Yedeklerim nerede, neye benziyor?**
Savepoint olmadan da açılabilen düz klasörler: `<yedek klasörü>\<oyun>\<tarih-saat>\` içinde kayıt dosyaları ve onları anlatan bir `backup.json`. Bir oyunun sayfasındaki *Yedekler* düğmesi klasörünü açar.

**Kaç yedek saklanır?**
Varsayılan olarak oyun başına 10 (Ayarlar'dan değiştirilebilir). Yenisi alınınca en eskisi silinir, bozuk diye işaretlenmemiş son yedek hariç.

**Oyunum listede yok.**
Arama kutusunun yanındaki yenile düğmesine bas ya da oyunu bir kez açıp kapat (kayıt klasörü ondan önce oluşmamış olabilir). Hâlâ yoksa *Oyun ekle* ile ekle.

**Tepsi simgesini gizledim, Savepoint'i nasıl açarım?**
Başlat menüsünden (ya da `savepointd.exe`'yi) tekrar çalıştır, çalışan kopya penceresini açar.

**Kaldırınca yedeklerim silinir mi?**
Hayır. Kaldırıcı sadece programı siler. Yedeklerin ve `%LOCALAPPDATA%\Savepoint` olduğu gibi kalır.

## Nasıl çalışır

| Parça | Görevi | Ne zaman çalışır |
|---|---|---|
| `savepointd.exe` | Tepsi simgesi, oyun algılama, yedek kuyruğu, yerel named pipe sunucusu | Her zaman, penceresiz |
| `savepointd.exe --scan` | Oyun manifest'ini indirir ve okur, bu bilgisayardaki oyunları bulur | Haftada bir, pencere açıldığında liste 10 dakikadan eskiyse ya da istenince, kısa ömürlü bir alt süreç olarak |
| `savepoint-ui.exe` | Pencere ([Tauri](https://tauri.app), düz HTML, CSS ve JavaScript) | Sadece açıkken |

## Kaynaktan derleme

Windows 10/11, MSVC araç zinciriyle [Rust](https://rustup.rs) 1.90 ya da üstü ("Desktop development with C++" içeren Visual Studio Build Tools) ve kurulum paketi için isteğe bağlı olarak [Inno Setup 6](https://jrsoftware.org/isinfo.php) gerekir. Node.js gerekmez.

```powershell
cargo build --release          # target\release\savepointd.exe ve savepoint-ui.exe
cargo test --workspace         # testler
.\scripts\package.ps1          # dist\: taşınabilir zip, kurulum, SHA256SUMS.txt
```

Ayrıntılar [CONTRIBUTING.md](CONTRIBUTING.md) içinde (İngilizce).

## Bilinen sınırlar

- Sadece Windows.
- Windows kayıt defterinde tutulan kayıtlar (bazı eski oyunlar) yedeklenmez.
- GOG Galaxy ve Xbox uygulaması üzerinden algılama henüz gerçek kullanımda az denendi, geri bildirimler memnuniyetle karşılanır.
- Sürüm dosyaları henüz kod imzalı değil (yukarıdaki SmartScreen notuna bak).

## Katkı

Hata bildirimleri, fikirler, çeviriler ve pull request'ler memnuniyetle karşılanır. Lütfen [CONTRIBUTING.md](CONTRIBUTING.md) ve [davranış kurallarını](CODE_OF_CONDUCT.md) oku. Güvenlik sorunları için: [SECURITY.md](SECURITY.md).

## Teşekkür

- Oyun kayıt konumları: mtkennerly'nin [Ludusavi Manifest](https://github.com/mtkennerly/ludusavi-manifest)'i, [PCGamingWiki](https://www.pcgamingwiki.com) verilerinden derlenir.
- Pencere: [Tauri](https://tauri.app). Kullanılan kütüphanelerin tam listesi ve lisansları her sürümde `THIRD-PARTY-LICENSES.txt` olarak gelir.

## Lisans

[MIT](LICENSE) © 2026 eneskocamaan
