# Sonraki oturum — güvenilirlik düzeltmeleri, Monty Bash ve arama

Kaydedildi: 5 Ekim 2026, Europe/Istanbul.
Kullanıcının istediği dönüş tarihi: **yarın, 6 Ekim 2026**.

**5 Ekim uygulama güncellemesi:** Kullanıcı işi öne aldı. Aşağıdaki metin ilk planın
kaydını korur; güncel uygulama durumu değildir. `vsh_bash` ve rapor düzeltmeleri uygulandı.
FFF deneyi sonrasında kontrollü ölçüm belirgin transaction kazancı göstermediği için
`fff-grep` kaldırıldı; mevcut `memchr` eşleştiricisi doğrudan kullanılıyor. Son karar ve
ölçüm sınırları [performans dokümanında](../docs/performance.md).
F04'te gözlenen taşıma yarışı engelleniyor; düşmanca
harici host yazıcıya karşı OS izolasyonu kullanıcı kararıyla ertelendi. Bu, kalıntı
yarışın tamamen çözüldüğü anlamına gelmez; mevcut kullanım kontrollü workspace şartını
korur. Ayrıntılar, yeniden açılma koşulları ve kalan doğrulamalar
[ertelenen OS izolasyonu planında](deferred/os_isolation.md); test kanıtları
[uygulama raporunda](../target/VSH_REMEDIATION_2026_10_05.md).

**FFF kaldırma doğrulaması (5 Ekim):** `memchr = "=2.8.3"` korundu; satır adapter'ı
yerel iterator ile değiştirildi ve matcher revision'ı execution digest'e işlendi.
Son kodda 307 Rust testi ve rebuilt native extension ile 254 Python testi geçti;
Python satır/branch coverage %100. Clippy, rustfmt, Ruff, ty, basedpyright ve strict
Zensical kontrolleri temiz; 39 doküman sayfası, 4.302 yerel link ve 59 Python snippet
doğrulandı. Generated llms/copy-as-Markdown içerikleri güncellendi. Loglar
`target/fff-removal-*.log` altında (yerel, Git'e dahil değil). Önceki matcher A/B
ölçümü tekrar çalıştırılmadı; Rust coverage, hosted CI ve temiz wheel consumer gate'i
bu dar değişiklik için yeniden çalıştırılmadı. Commit/push/release yapılmadı.

**Hatırlatma durumu:** Bu ortamda bağımsız hatırlatıcı oluşturma aracı bulunmadığı için zamanlanmış bildirim kurulmadı. Bu dosya kalıcı çalışma notudur; otomatik hatırlatıcı değildir. Henüz bir saat seçilmedi.

## Önce mevcut sorunlar

Başlangıç kanıtı: [4 Ekim native inceleme raporu](../target/VSH_EXTERNAL_REVIEW_ASSESSMENT_2026_10_04.md). Reproduksiyonlar `target/external_review_probes_2026_10_04/` altında; bunlar yerel/ignored kanıtlardır, clone ile gelmezler. Kullanıcının üç kaynak dosyası Downloads altında `REVIEW.md`, `SOURCE_EVIDENCE.md`, `probe_results.json`.

İlk gündem aşağıdaki bulguları çözmek; yeni özellikler bunların önüne geçmeyecek:

1. **F01:** Yeni invocation ile aynı transaction'ın replay edilmesini ayır. Aynı dosyanın art arda okunması `duplicate transaction` vermemeli; aynı commit yetkisi yine tek kullanımlık olmalı.
2. **F02:** Agent-facing sonuç doğrulaması commit sonrası sürpriz hata üretmemeli. `set`/`frozenset` ve `1`/`"1"` mapping anahtar çakışmaları; hook'lu/hook'suz yollar birlikte kapsanmalı. Commit bilgisi cevap hatası yüzünden kaybolmamalı.
3. **F04:** Dış yazıcının pinlenmiş parent directory'yi workspace dışına taşıdığı durumda gerçek containment sözleşmesini ve platform çözümünü belirle. Mevcut native probe, dışarı taşınmış dizine yazma sonrasında `RecoveryRequired` üretiyor. Guest'in tek başına host erişimi kazanması değildir; rename yetkili eşzamanlı dış yazıcı gerektirir. Sonradan hata tespiti önleme değildir.
4. **F03/F07:** Host snapshot/materialization işini erken count/byte/memory/deadline/cancellation sınırlarıyla kapsa. Guest read bütçesi host canonicalization bütçesinin yerine geçmiyor; snapshot çocukları limit kontrolünden önce sınırsız toplanmamalı.
5. **F08:** Hook ile hazırlanmış durable artifact hook'suz runtime'da yeniden açıldığında eski review şartı sessizce kalkmamalı. Yetkili host migration'ının sözleşmesini açıkça belirle; tüm runtime digest'lerini körlemesine eşitleme.
6. **F05/F06:** Model-facing MCP yetkilerini host'a sabitle; response/evidence tekrarını azalt; token/agent-loop maliyetini ayrı ölç.

Çalışma yöntemi: güncel HEAD/worktree kontrolü → küçük sentetik fixture ile repro → hedefli düzeltme → native regresyon → ilgili geniş test/gate. Gerçek kullanıcı dosyalarında adversarial test veya gerçek model/API harcaması yok. Commit/push/release için bu not ayrıca yetki vermiyor.

## Yeni fikir 1: Monty toolset içinde `vsh_bash`

**Karar:** Tasarlamaya değer; henüz uygulanmadı. Amaç, Monty programının gerektiğinde mevcut bounded Bash backend'ini çağırıp aynı sanal filesystem üzerinde çalışmaya devam edebilmesi.

Hedef akış:

```text
Monty programı
  → vsh_write / pathlib ile sanal değişiklik
  → vsh_bash(...) aynı aktif VirtualFs'i görür
  → Monty aynı değişiklikleri okuyup doğrular
  → tek canonical transaction → tek policy/review/commit lifecycle
```

Örnek yalnızca hedef kullanım taslağıdır; mevcut public API değildir:

```python
vsh_write('/workspace/message.txt', 'hello\n')
result = vsh_bash('cat message.txt', cwd='/workspace')
assert result['stdout'] == b'hello\n'
```

### Korunacak sözleşmeler

- Host `/bin/bash`, subprocess çalıştırma yetkisi veya native executable mount edilmeyecek. Mevcut VSH Bashkit profili kullanılacak.
- Nested `Runtime.run`, yeni snapshot, bağımsız inner transaction veya inner commit olmayacak. Filesystem değişiklikleri aynı overlay/read-set/write-set/effect ledger içinde kalacak.
- Host'un Bash etkinleştirmesi gerekecek; guest kendi kendine backend/capability açamayacak.
- Her çağrı fresh shell state ile başlamalı; dosyalar paylaşılır, gizli shell değişkenleri/functions/cwd sonraki çağrıya taşınmaz. `cwd` açıkça virtual namespace'e bağlı olmalı.
- Policy/protected paths/path mapping kontrolleri ortak gateway'den geçmeli. Bash efektlerinin kaynağı ayırt edilebilir kalmalı.
- Bütçeler her nested çağrıda sıfırlanmayacak. Outer programın toplam süre, IO, çıktı ve kaynak limitleri; inner worker'ın kendi sert limitleriyle birlikte uygulanmalı.
- Worker crash, protocol/profile/namespace ihlali, cancellation ve budget aşımı outer transaction'ı da başarısız kılmalı; Monty `try/except` bunu tekrar approvable hale getirememeli.
- Mevcut Bash final nonzero davranışı varsayılan olarak korunmalı: partial virtual writes ile actionable artifact üretilmemeli. Başka semantics istenirse ayrıca tasarlanmalı; composition bahanesiyle mevcut güvenlik profili gevşetilmemeli.
- Başarılı dönüş bounded exit code + byte-authoritative stdout/stderr taşımalı. Monty içinde desteklenen temsil seçilmeli; Python SDK `BashResult` nesnesinin doğrudan guest'e taşınabildiği varsayılmamalı. Yukarıdaki dict şekli henüz kesin API kararı değil.
- Outer transaction binding, kullanılan Bash sürümü/profili/etkin yetkiler ve execution evidence'ı da kapsamalı. Ana dilin Monty olması inner backend konfigürasyonunu bağın dışında bırakmamalı.
- Monty suspension → parent Bash dispatch → filesystem RPC sırası deadlock ve reentrant lock açısından sınanmalı. İç içe worker çağrıları için ayrı, sınırsız pool oluşturulmamalı.

### Kabul testleri

1. Monty write → Bash read; Bash write/rename/delete → Monty read/list/search.
2. Tek final diff ve tek hook lifecycle; preview sırasında kullanıcı workspace'inde değişiklik yok.
3. Korunan dosyaya inner Bash erişimi engelleniyor; guest hatayı yakalasa da terminal ihlal geçersiz artifact üretiyor.
4. Birçok küçük Bash çağrısı toplam bütçeyi aşamıyor; stdout/stderr kayıpsız ve bounded.
5. Inner timeout/cancellation/crash sonrası commit yok; worker/handle sızıntısı yok.
6. Mevcut Monty-only median/p95, memory ve worker startup davranışında ölçüsüz ek maliyet yok.

İlgili bileşenler: `vsh-monty` guest registration/dispatch, `vsh-execution` gateway/budget, mevcut Bash worker host adapter'ı, outer runtime binding/evidence. Önce dar teknik spike ve kaynak sahipliği tasarımı; mevcut Bash interpreter'ını yeniden yazmak veya generic plugin framework eklemek kapsam dışı.

## Yeni fikir 2: `fff` ile arama hızlandırma

**Karar:** Benchmark ve entegrasyon incelemesine alındı. Henüz dependency eklenmedi; daha hızlı olduğu doğrulanmadı. `vsh_search` hemen değiştirilmez, `fff` adında yeni public tool otomatik olarak eklenmez.

### Kaynak incelemesinden çıkanlar

- FFF; uzun ömürlü indeks, fuzzy path/content search, literal/regex seçenekleri, watcher ve cache kullanan bir Rust arama projesi. Hız karşılaştırmalarının önemli motivasyonlarından biri tekrar tekrar CLI başlatmamak ve indeks/cache'i yeniden kullanmak. Bunlar upstream'in iddia ve tasarım açıklamalarıdır, VSH benchmark'ı değildir. [Resmî README](https://github.com/dmtrKovalenko/fff/blob/main/README.md)
- Tam `fff-search`/`FilePicker` yüzeyi host directory scan/watch lifecycle'ı taşıyor. Doğrudan workspace'e bağlamak aktif VSH overlay'i görmeme, policy-hidden dosyaları indeksleme ve host ile snapshot arasında farklı sonuç üretme riski taşır. Bu VSH'ye entegrasyon çıkarımıdır; hazır bir VSH adapter'ı olduğu iddia edilmiyor. [FilePicker kaynağı](https://github.com/dmtrKovalenko/fff/blob/main/crates/fff-core/src/file_picker.rs)
- Ayrı `fff-grep` bileşeni line-oriented byte-slice search sunuyor; library açıklamasında file/reader/mmap search değil `search_slice` desteklediğini belirtiyor. Bu, VSH'nin yetkilendirilmiş byte'ları sağlayıp yalnız eşleştirmeyi devretmesi için daha dar bir aday. **Bu alt bileşeni kullanmak tam FFF indeks performansını otomatik getirmez.** [fff-grep kaynağı](https://github.com/dmtrKovalenko/fff/blob/main/crates/fff-grep/src/lib.rs)
- Tam core'un dependency maliyeti de var: git2, LMDB/heed, notify, mmap, paralellik ve matching bileşenleri. Yalnız matcher ihtiyacı için hepsini almak varsayılan karar olmamalı. [Core manifest](https://github.com/dmtrKovalenko/fff/blob/main/crates/fff-core/Cargo.toml)
- İncelenen `main` manifesti `0.11.0`, README Rust kurulum örneği ise `0.6` gösteriyor. Bunlar latest published/safe sürüm tespiti değildir. Entegrasyon anında gerçek release/crates.io sürümü, bakım durumu, lisans, audit/deny ve platform desteği doğrulanıp yalnız kabul edilen sürüm exact-pinlenmeli.

### Mevcut VSH ile fark

Şu an `vsh_search`, Rust içinde aktif VFS'ten literal içerik arıyor; her sorguda `rg` subprocess'i başlatmıyor. UTF-8 dosyalarda satır başına ilk eşleşmeyi, bir tabanlı Unicode column'u ve bounded sonucu döndürüyor; limitte traversal duruyor. Case-insensitive davranışı Unicode lowercase tabanlı. [Mevcut kod](../crates/vsh-monty/src/tools.rs), [sözleşme](../docs/integrations/monty-tools.md).

Dolayısıyla FFF-vs-ripgrep hız grafiği VSH karşılaştırmasının yerine geçmez. Fuzzy filename search de mevcut literal content search ile aynı ürün davranışı değildir. Mevcut default'u “bulamazsa fuzzy” davranışına çevirmek uyumsuzluk olur.

### Güvenli entegrasyon yaklaşımı

1. Önce mevcut search hot path'ini ölç; maliyet filesystem traversal/materialization mı, allocation/Unicode dönüşümü mü, yoksa matching mi ayır.
2. Aynı sözleşmede mevcut matcher, dar `fff-grep` adapter'ı ve gerekirse uygun düşük maliyetli literal matcher seçeneğini karşılaştır. Yeni dependency'yi ölçüm öncesi kalıcılaştırma.
3. Veriyi yalnız policy-aware VSH gateway'den al. Host filesystem path'ini arama motoruna verip sonradan sonuç filtrelemek yeterli değil.
4. İndeks gerekli ve faydalı çıkarsa base snapshot + aktif overlay birleşimini temsil et: yeni dosyalar görünsün; tombstone'lar ve eski rename yolları kaybolsun; güncel content version kullanılsın.
5. Cache/index ömrü ve anahtarı tenant/workspace, snapshot/content version, policy ve transaction overlay ile uyumlu olmalı. Cross-workspace içerik veya ranking/frecency sızıntısı olmamalı. Kalıcı frecency varsayılan olarak kullanılmamalı.
6. Cache hit, read authorization veya stale dependency kaydını atlamamalı. “Eşleşme yok” ve index prefilter ile elenen dosyalar da sonucu etkiler; gerekli content/directory gözlemleri kanıtlanmalı. Sadece dönen sonuç dosyalarını read-set'e yazmak yeterli değildir.
7. Index build/update, host RAM/CPU/disk, sonuç ve query maliyetleri bounded olmalı. Guest başına büyük host kopyası veya her transaction'da tam indeks kurulması kabul edilmez.
8. Ek fuzzy/path search istenirse literal `vsh_search` sözleşmesinden açıkça ayrılmalı; public isim/parametre kararı ölçüm ve kullanıcı ihtiyacı netleştikten sonra verilmeli.

### Ölçüm planı

- Aynı snapshot, aynı izinler, aynı literal sorgular, aynı max_results, aynı Unicode/ordering davranışı. Regex ve fuzzy ayrı kategoriler.
- Soğuk başlangıç: traversal + index build + ilk query dahil. Sıcak kullanım: 1/10/100 ardışık query ve amortisman noktası.
- Küçük/orta/büyük workspace; çok küçük dosya, az büyük dosya, geniş dizin; no-match/common/rare queries; Unicode, binary ve protected-file örnekleri.
- Overlay edit/rename/delete sonrası sonuç doğruluğu ve incremental-update maliyeti; birden çok eşzamanlı workspace.
- Median/p95 query ve end-to-end preview latency; peak RSS ve retained index bytes; materyalize edilen içerik, CPU ve dependency/binary boyutu.
- VSH mevcut erken-result short circuit'ini koruyan karşılaştırma. Tam dosya tarayan bir adayın farklı sonuç sayısıyla hızlı gösterilmemesi.
- Sonuçlar host fixture'larında değil, sentetik snapshot/overlay testleriyle doğrulansın; performans için gereken sentetik host fixture varsa yalnız disposable temp root altında olsun.

**Seçim kriteri:** Anlamlı end-to-end kazanç + kabul edilebilir bellek/başlangıç maliyeti + eşdeğer yetki/kanıt/arama semantics. Kazanç yalnız matching mikrobenchmark'ında kalıyorsa veya VSH'nin güvenlik sınırını bozuyorsa entegrasyon yapılmaz.

## Kısa karar özeti

- Önce doğrulanmış 0.6.0 bulguları.
- `vsh_bash`: olumlu; aynı snapshot ve tek transaction içinde composition hedefleniyor.
- `fff`: olumlu araştırma adayı; tam host indeksleyicisi yerine dar matcher veya VFS-aware indeks tercih edilecek, karar benchmark'a bağlı.
- Bugün yalnız bu not ve kaynak değerlendirmesi eklendi. Feature implementasyonu, dependency değişikliği ve zamanlanmış bildirim yok.

### Sonraki kullanıcı isteğiyle ölçüm — 5 Ekim

[FFF/VSH benchmark raporu](../target/FFF_VSH_SEARCH_BENCHMARK_2026_10_05.md) ve [ham sonuçlar](../target/fff_search_benchmark_2026_10_05/results.json) eklendi. FFF 0.11.0, VSH 0.6.0, Apple M1/8 GiB: 200/2.000/5.000 adet 4 KiB dosyada sıcak SDK sorgularında FFF yaklaşık 18–31× hızlı; VSH'de aynı transaction içinde 15 sorguya batching yapılınca fark yaklaşık 6–8×. Büyük sette koşular arası oynaklık var. Bu bir saf matcher veya eşdeğer güvenlikli adapter benchmark'ı değil. FFF yalnız ignored benchmark klasörüne kuruldu; ürün dependency'si ve implementasyon değişmedi. Cold VSH capture/blob-store maliyeti ayrıca profile edilmeli. Önceki "henüz ölçülmedi" notunun güncel durumu budur; güvenli VFS adapter'ı için kazanç hâlâ ayrı doğrulanacak.
