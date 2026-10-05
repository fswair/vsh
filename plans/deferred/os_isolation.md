# Ertelenen özellik: VSH box katmanı için OS izolasyonu

Durum: **ertelendi; implementasyon yetkisi verilmedi**.
Karar tarihi: 5 Ekim 2026.
Öncelik: mevcut transaction çekirdeği ve Python/Rust API'lerini stabilize etmek.
Sürüm/tarih taahhüdü: yok. Bu belge yeni bir public API veya yayımlanmış garanti değildir.

Kullanıcı OS izolasyonu fikrinin korunmasını, fakat şu an uygulanmamasını istedi.
Mevcut kullanım sözleşmesi host tarafından kontrol edilen workspace'tir. F04'teki
belirli yarışın önlenmesi, düşmanca harici yazıcıya karşı genel containment kanıtı
sayılmaz. Erteleme, bu kalıntı riski teknik olarak çözülmüş yapmaz.

## 1. Amaç ve neden ayrı bir katman?

VSH'nin işi, guest programını sanal filesystem üzerinde çalıştırmak, gerçek etkileri
kaydetmek, canonical diff üretmek, policy/review uygulamak ve exact transaction'ı
revalidate ederek recoverable commit yapmaktır. Monty ve Bashkit bunun execution
frontend'leridir; kernel veya tenant izolasyonunun yerini almazlar.

Gelecekteki box/SaaS katmanı ise güvenilmeyen kullanıcılar arasında dosya, process,
network ve kaynak yetkisini ayırmalıdır. Kullanıcı bir workspace, saklama alanı ve
RAM/CPU bütçesi görür; arkada shared Linux host üzerinde izole execution birimleri
bulunabilir. VSH çekirdeği tek başına orchestrator/container sistemi hâline gelmemeli.

Başarı senaryosu: tenant A'nın guest'i, tool'u veya erişebildiği process'i tenant B'nin
workspace'ini, journal'ını, content blob'larını, worker'ını veya credentials'ını
göremez/değiştiremez; A'nın taşma/OOM/iptal senaryosu B'nin güvenlik sınırını aşamaz.

Bu planın varlığı; Kubernetes, SSH, GUI, host binary execution, network veya yeni
crate ekleme yetkisi değildir. Bunlar gerekli olduklarında ayrı scope kararı ister.

## 2. Başlangıç problemi: F04

VSH bir parent directory'yi açıp kimliğini doğruladıktan sonra başka bir host process
bu directory'yi workspace dışına taşıyabilir. Açık directory handle aynı inode'u
göstermeye devam eder. Sonraki handle-relative write, o sırada dışarı taşınmış
directory'de gerçekleşebilir; sonradan `Stale`/recovery hatası görmek yazıyı geri almış
veya baştan engellemiş olmak değildir.

Mevcut düzeltme, revalidation sonrası ve mutation öncesi parent kimliğini tekrar
kontrol eder. Sentetik repro artık dış dosya oluşturmadan `Stale` oluyor. Ancak son
kontrolle syscall arasındaki başka bir taşıma hâlâ genel modelin dışında değildir.

Bu nedenle ilk ilke **eşit yetkili düşmanca yazıcıyı aynı namespace'in sahibi yapmamak**.
Ek `stat`, advisory lock veya pathname normalization bu sahiplik sorununu tek başına
çözmez. Workspace root'unun ve onu taşıyabilecek üst directory'lerin yetkileri de
hesaba katılmalıdır; yalnız leaf dosyaya izin koymak yeterli değildir.

Mevcut kanıt: [yerel remediation raporu](../../target/VSH_REMEDIATION_2026_10_05.md),
[commit testleri](../../crates/vsh-commit/src/tests.rs),
[threat model](../../docs/threat-model.md). `target/` altındaki loglar ignored/local'dır;
clone ile gelmez. Durable testler ve kaynak kod asıl tekrar üretim yüzeyidir.

## 3. Tehdit modeli

| Aktör/olay | Hedeflenen davranış |
| --- | --- |
| Untrusted Monty/Bash programı | Yalnız VFS gateway; gerçek mount, host env veya executable yetkisi yok |
| Başka tenant'ın process'i | Workspace/data/IPC/PID erişimi ve rename yetkisi yok |
| Aynı tenant'ın ileride eklenen host tool'u | Committer yetkisini miras almaz; ayrı dar yetki ve staged output |
| Aynı host'ta sıradan kullanıcı/process | Workspace ve üst directory'ler üzerinde izinsiz taşıma/yazma engellenir |
| İzolasyon kurulumu eksik/uyumsuz | Guest başlamaz; ilan edilen güvenli profile sessiz fallback yapılmaz |
| Yetkili host admin, host kernel/hypervisor kompromisi | Güven sınırının dışında; aynı host üzerindeki container ile çözülmüş sayılmaz |
| Donanım/shared-kernel yan kanalları | İlk profile kapsamında yok; hedef müşteri gerektirirse ayrı VM sınıfı değerlendirilir |

“Hostile external writer” sözü mutlaka hangi yetkiye sahip aktör anlamına geldiğini
belirtmelidir. UID, mount namespace veya microVM seçimi trusted host admin'e karşı
otomatik koruma sözü değildir. Ayrı VM, daha güçlü tenant kernel sınırı için adaydır;
kötü niyetli hypervisor'a karşı garanti değildir.

## 4. Authority ayrımı

```text
Authenticated API / control plane
    │ host-selected tenant, workspace, policy, quota, isolation profile
    ▼
Minimal trusted launcher / supervisor
    │ creates tenant-owned execution environment and scoped IPC
    ▼
VSH runtime + trusted commit authority
    ├── private workspace volume: sole mutation authority
    ├── trusted journal/blob/approval storage: unavailable to guests
    └── bounded gateway IPC
          ├── Monty worker: no workspace mount
          └── Bashkit worker: no workspace mount
```

Control plane credentials ve global mount/root yetkisi runtime/guest'e taşınmaz.
Launcher yalnız gerekli setup işlemlerini yapar; modelin verdiği raw path/UID/mount
argümanlarını çalıştıran genel bir privileged RPC servisi olmaz. Native runtime kendi
tenant'ının alanını yönetir; bütün tenant volume'larına sahip bir commit process'i
varsayılan mimari yapılmaz.

VSH transaction kimliği, native policy, hook scope, stale kontrolü, reservation,
journal ve recovery aynen korunur. OS isolation, onaysız transaction'ı onaylamaz;
judge kararı kernel izni veya yeni filesystem capability üretmez.

## 5. Storage ve F04'ün önlenmesi

İlk değerlendirme adayı Linux üzerinde tenant/workspace başına host-controlled private
storage ve ayrı erişim kimliğidir. Volume/directory'nin bulunduğu parent da başka
tenant'a writable değildir. Aynı writable backing tree başka process'e mount edilmez.

Mount namespace mount görünümünü ayırır; bind mount aynı backing dosyaları paylaşabilir.
Bu nedenle private mount propagation tek başına veri kopyası, immutable snapshot veya
harici host writer engeli sayılmaz. Host ownership, mount topology ve handle geçişleri
birlikte test edilmelidir. [Linux mount namespace manual](https://man7.org/linux/man-pages/man7/mount_namespaces.7.html)

İki kullanım profili birbirinden ayrı tutulur:

1. **Managed box:** Private storage'ın tek mutation authority'si trusted committer'dır.
   Dosya import/export, tenant auth ve quota kontrolü olan host hizmetinden geçer.
   Untrusted process volume'u veya parent'ını rename edemez.
2. **Local existing directory:** Kullanıcının editor/terminal/background tool'ları aynı
   directory'de yetkili olabilir. Bu otomatik olarak managed isolation değildir.
   Stale/revalidation kontrolleri çalışır ama düşmanca eşzamanlı writer containment
   garantisi verilmez. Güçlü garanti istenirse private staging volume'a import edilir;
   dış directory'ye export ayrı kontrollü işlem olur, yeni bir atomiklik iddiası olmaz.

Read-only export bile hassas içerik sızıntısı açısından ayrı yetkilendirilir. Symlink,
hard-link, mount alias ve önceden açılmış descriptor üzerinden dolaylı writable erişim
test edilmeden “tek yazıcı” kabul edilmez. Bir cooperating mutex, uncooperative writer'ı
engellemiş sayılmaz. Workspace'in taşınması/yedeklenmesi bakım operasyonuysa admission
durdurulur ve aktif commit/recovery tamamlanmadan storage topology değiştirilmez.

## 6. OS kontrol katmanları — adaylar, bugün seçilmiş API'ler değil

| Kontrol | Sorumluluğu | Tek başına sağlamadığı şey |
| --- | --- | --- |
| Ayrı UID/ownership ve dar filesystem izinleri | Tenant write/read authority ayrımı | Aynı yetkiyi paylaşan process'ler veya privileged admin'e karşı koruma |
| Mount namespace + explicit propagation | Görünen mount'ları sınırlamak | Aynı inode/backing volume'u paylaşan dış writer'ı engellemek |
| PID/IPC/network ayrımı | Process/IPC görünürlüğü ve bağlantı yüzeyini daraltmak | Filesystem policy veya tenant auth |
| No-new-privileges, dar capability seti, syscall filter | Child'ın privilege/syscall yüzeyini azaltmak | Native VSH policy ve canonical-diff güvenliği |
| Landlock / uygun LSM profili | Process'in filesystem erişimine ek kısıt | Önceden geçirilmiş her handle'ı geri almak; tüm host process'lerini kısıtlamak |
| cgroup v2 + admission + storage quota | Process tree CPU/RAM/PID ve kapasite yönetimi | İşlemin semantik güvenliği veya transaction atomikliği |
| Ayrı VM/microVM | Gerekiyorsa tenant için ayrı kernel sınırı | Trusted host/hypervisor kompromisini çözmek veya ücretsiz kaynak |

Landlock'ta bazı haklar descriptor açılırken ilişkilendirilir; sandbox kurulmadan önce
açılan veya IPC ile geçirilen descriptor'lar ayrıca denetlenmelidir. ABI'lere göre
rename/link/truncate desteği değişebilir. İlan edilen strict profile için gereken
kernel/ABI yoksa başlatma reddedilir; “desteklenen kadar uygula” yaklaşımı aynı güvenlik
profile adıyla sunulmaz. [Resmî Landlock dokümanı](https://docs.kernel.org/userspace-api/landlock.html)

cgroup CPU/memory/PID controller'ları, VSH guest bytecode/heap/I/O limitlerinden ayrıdır.
Supervisor, runtime ve iki worker'ın toplamı ölçülür; parent RSS'yi tek başına RAM maliyeti
gibi raporlamak yasaktır. Swap, tmpfs, page cache, journal/blob büyümesi, disk dolması ve
FD/thread sınırları da profile tasarımında değerlendirilir. Controller kullanılabilirliği
ve delegation deployment'ta doğrulanır. [Resmî cgroup v2 dokümanı](https://docs.kernel.org/admin-guide/cgroup-v2.html)

Buradaki kaynaklar primitive davranışları için referanstır; VSH entegrasyonunun doğru
olduğunu kanıtlamaz. Dokümanlar 5 Ekim 2026'da incelendi; kernel/doc dalı release pin'i
olarak seçilmedi. Implementasyon başında support matrix ve güncel güvenlik durumu tekrar
doğrulanacak; Rust dependency eklenecekse bakımlı exact sürüm ve audit/deny şartı sürecek.

## 7. Lifecycle ve transaction protokolü

1. Host kimliği doğrular; tenant/workspace mapping'i model argümanlarından değil trusted
   metadata'dan çözer. Rate/concurrency/storage admission uygular.
2. Launcher private storage, process kimliği ve OS profile'ını kurar. Gereksiz inherited
   FD/env/cwd/credentials temizlenir; failure guest'ten önce fail-closed olur.
3. Supervisor gerçekten uygulanan profile ve volume kimliğini host tarafında doğrular.
   Basit bir guest `isolated=true` cevabı veya konfigürasyon digest'i attestation değildir.
4. Runtime mevcut VSH snapshot → execution → diff → policy → review yolunu kullanır.
   Monty/Bash yalnız gateway ile konuşur; worker'a storage root/commit endpoint verilmez.
5. Commit authority tek kullanımlı exact transaction ve geçerli reviewer yetkisini
   doğrular. Workspace'e ait commit serialization ve stale kontrolleri korunur.
6. Cancel/timeout guest process tree'yi sınırlar. Durable commit'e girildiyse response
   gerçek committed/recovery durumunu anlatır; sırf HTTP isteği iptal oldu diye journal
   silinmez veya transaction yeniden yürütülmez.
7. Crash/OOM/disk-full sonrası yalnız trusted recovery owner journal'ı işler. Sahipliği
   belirsiz inode/artifact'ler otomatik temizlenmez; quarantine/operator yolu gerekir.
8. Box kapanırken active request'ler drain edilir, handles/pool temizlenir, retention
   uygulanır. Başka tenant için pool reuse ancak reset/secret/FD/cache sınırı kanıtlanırsa;
   ilk sürümde tenant'lar arasında mutable worker/runtime paylaşımı yok.

Idle pool amortismanı ile güvenlik birbirine karıştırılmaz. Bir transaction için fresh
shell/interpreter state gereksinimi ve aynı transaction içinde `vsh_bash` ortak overlay
sözleşmesi değişmez. Snapshot caching veya FFF index başka bir feature'dır; tenant,
content version, policy ve overlay key'leri olmadan ortak indeks eklenmez.

## 8. Network, binary, SSH ve GUI sınırı

İlk OS profile'ında guest network ve host executable yüzeyi genişlemez. Gelecekte
allowlisted binary ihtiyacı doğarsa ayrı executable policy, syscall/resource profile,
environment allowlist ve staged filesystem output kontratı gerekir. Gerçek binary'ye
doğrudan writable workspace vermek VSH review/commit hattını bypass edebilir.

LLM/BYOK çağrıları trusted gateway tarafında tutulur; key guest env, process argümanları,
stdout veya genel workspace dosyası üzerinden taşınmaz. Judge'a gönderilen içerik için
mevcut path-bound sharing izni ve evidence bütçesi korunur. Network gerekiyorsa tenant
auth, egress policy ve bütçeler ayrı özellik olarak tasarlanır.

SSH/GUI bir transport/UX kararıdır, yetki yükseltme değildir. “VSH kontrollü düzenleme”
ile “kullanıcının doğrudan writable OS shell'i” ayrı profiller olmalıdır. İlkinin verdiği
approval garantisi ikincisine otomatik taşınmaz; read-only viewer veya staged edit/explicit
import modelleri önce değerlendirilir. Kernel/desktop/SSH implementasyonu bu planın v0'ı değil.

## 9. Kademeli implementasyon planı — yeniden aktive edilince

| Aşama | İş | Çıkış kanıtı |
| --- | --- | --- |
| P0 — sözleşme/probe | Hangi attacker UID/capability/mount yetkileriyle kapsamda? Desteklenen Linux/kernel/ABI ve filesystem matrix'i; F04 dahil adversarial harness | Approved threat model; olumlu/olumsuz kontrol senaryoları; desteklenmeyen profile açık ret |
| P1 — dar launcher spike | Tek tenant private storage, dar process identity, temiz FD/env/cwd ve bounded IPC; mevcut VSH'ye yalnız gerekli handles | Setup failure, parent rename, inherited handle ve peer-process testleri |
| P2 — kaynaklar/lifecycle | Process-tree quotas, admission, idle pool, crash/OOM/cancel/disk-full ve journal retention | Sızıntısız teardown; truthful commit/recovery; ölçülmüş CPU/RAM/PID/disk maliyeti |
| P3 — hostile multi-tenant | İki tenant ve malicious peer; mount/UID/IPC/network/ptrace/cache/credential probe'ları | Tenant B ve dış sentinel'larda hiçbir yetkisiz okuma/yazma; isolation regression CI |
| P4 — ürün adaptörü | Authenticated box service, supported profile raporlama, operation logs, provisioning/recovery runbook | Mevcut SDK davranışı korunur; profile düşürme fail-closed; deployment/rollback doğrulaması |
| P5 — kapasite/rollout | Cold/warm benchmark, contention, tail latency, capacity admission; sınırlı rollout | Ölçüme dayalı maliyet ve kabul eşiği; rollback ve incident exercise |

P0/P1 sonuçları yeterli değilse microVM veya daha dar ürün profile'ı değerlendirilir;
hazır bir mekanizmayı seçmiş olmak feature'ın güvenli olduğu sonucu değildir.
Her aşama sonunda gerçek kaynak diff'i ve negatif senaryolarla bağımsız inceleme gerekir.

Beklenen temas noktaları: mevcut [runtime](../../crates/vbash/src/runtime.rs),
[committer](../../crates/vsh-commit/src/committer.rs),
[Monty parent](../../crates/vsh-monty/src/worker.rs),
[Bash parent](../../crates/vsh-bash/src/host.rs), worker setup/protocol ve deployment
testleri. Yeni supervisor/crate/API isimleri şimdi sabitlenmez. OS-specific bootstrap
taşınabilir `vsh-types`/VFS çekirdeğine sızmaz; gerekirse optional deployment bileşeni olur.

## 10. Güvenlik kabul matrisi

- F04: root/parent/intermediate directory rename ve swap; revalidation, intent, syscall
  öncesi/sonrası bütün kritik noktalarda adversarial scheduling; outside sentinel hash ve
  inode kontrolü. Yalnız exception/state kontrolü başarı ölçütü değildir.
- Aynı uid, farklı uid, namespace içi/dışı, writable bind alias, inherited FD, hard-link,
  symlink, proc üzerinden descriptor ve IPC ile handle geçişi. Kapsam dışı privileged
  admin senaryosu “engel başarısız” yerine dürüstçe farklı threat sınıfı olarak kaydedilir.
- Positive control: aynı adversarial taşıma yetkisiz peer'de engellenirken normal
  create/update/rename/remove/chmod, hooks ve recovery çalışmalıdır.
- Approval replay, expired grant, profile downgrade, tenant-id/workspace-id karışması ve
  cross-tenant artifact reuse reddedilir. İmza/digest tek başına auth yerine geçmez.
- Monty ve nested/top-level Bash aynı sınırda kalır; `.env`, token, private journal ve
  diğer tenant path'leri sonuç/exception/stdout/search index'ine sızmaz.
- Crash/kill/OOM/disk-full/worker timeout hem commit öncesinde hem durable commit içinde
  fault injection ile denenir. Reservation/journal kaybı veya sahte rollback başarısı yok.
- Eksik kernel feature, yetersiz launcher yetkisi ve bozuk profile kurulumu guest'ten önce
  reddedilir; görünmez “best effort” downgrade yok.
- Cleanup tenant B'ye dokunmaz; tenant A'nın mount/FD/worker/credential/cache'i reuse'da
  kalmaz. Bir testin host sentinel'ını değiştirmesi bile gate'i başarısız yapar.

Bu testler yalnız disposable VM/volume/synthetic fixture üzerinde çalıştırılır. Geliştirici
home directory'si veya gerçek müşteri workspace'i adversarial test hedefi yapılmaz.

## 11. Performans ve maliyet kabulü

Henüz hız/maliyet rakamı veya “container'dan ucuz” garantisi yok. Örnek başına ölçülecekler:
cold provision + ilk işe hazır olma, warm dispatch p50/p95/p99, preview ve commit ayrı,
process-tree peak/steady RSS ve cgroup bellek kullanımı, CPU zamanı, FD/PID/thread sayısı,
disk/journal/blob büyümesi, idle box maliyeti ve tenant contention altında başarı oranı.

Karşılaştırma: aynı fixture ve aynı VSH semantics ile controlled-host baseline, aday OS
profile ve gerekliyse VM profile. Engine/model/token tasarrufu ayrı ölçülür. Derleme,
indeks warm-up ve cold-start süreleri karıştırılmaz; cache koşulları kaydedilir.
Kabul eşikleri ilk baseline'dan sonra sahibiyle belirlenir; düşük maliyet için isolation,
policy, read-set veya durability kontrolleri kaldırılmaz.

## 12. Ne zaman yeniden açılır?

- Aynı host'ta birbirine güvenmeyen tenant/process çalıştırma taahhüdü verileceğinde.
- Host binary/network/SSH/GUI gibi guest authority'sini genişleten bir özellik gündeme geldiğinde.
- Müşteri controlled-workspace şartını sağlayamıyor ama güçlü containment istiyorsa.
- Mevcut guard'ları atlayan yeni reproducible yarış veya yanlış güvenlik vaadi bulunursa.

Şu an yapılmayacaklar: `sandbox=True` gibi gerçekte uygulanmayan public flag, yeni
dependency/paket, container manifest'i, privileged daemon, Kubernetes kurulumu ve kernel
sürümü pin'i. Bunları implementasyon başlamadan gereksiz public surface'e dönüştürmeyiz.

## Ek A — Bugünkü işte kalanlar

OS izolasyonu ertelendi. Bilinen remediation maddeleri ve ilk `vsh_bash`/FFF deneyi için
yerel testlerde ek fonksiyonel blocker görülmedi; bu “hatasız” veya “release hazır” kanıtı
değildir. FFF kaldırılmadan önce 254 Python ve 305 Rust testinin geçtiği kayıtlar var; Python satır/
branch coverage %100, Rust satır %84,94, fonksiyon %77,92, region %85,26. Lint/type/rustdoc/
Zensical ve dependency gate kayıtları geçti; eski izinli atomic-polyfill bakım uyarısı sürüyor.

Yayın öncesi kalan doğrulama: son geniş diff'e yeni bağımsız review, Linux/Windows dahil
hosted CI, temiz ortamda wheel/crate consumer smoke ve sürüm/migration/release notları.
Yerel testler rebuilt extension/matching workers ile çalıştı; yayımlanmış wheel değildir.
Commit/push/release için henüz yeni talimat yok; bu belge bunları tetiklemez.

Sonraki kontrollü matcher A/B ve maliyet ayrıştırması tamamlandı. Doğrudan `memchr`
ile paired median transaction farkları −%0,73 ile +%0,33 arasında kaldı; belirgin FFF
avantajı görülmedi. Kullanıcı onayıyla `fff-grep` kaldırıldı, mevcut `memchr` hızlandırması
korundu. Ayrıntılar ve ölçüm sınırları [performans dokümanında](../../docs/performance.md).
Capture/blob maliyeti ayrı optimizasyon konusu; gerçek agent-loop token/judge maliyeti
henüz ölçülmüş değil. Bu karar OS izolasyonu kapsamını değiştirmez.

## Ek B — İlk FFF ölçümünün tarihsel özeti

Bu ilk değerlendirmede yeni benchmark çalıştırılmadı; önceki ham JSON'lar yeniden
hesaplandı. Sonraki kontrollü ölçüm ve kaldırma kararı Ek A'dadır. Üç round'da ilk
`repeat=0` warm-up hariç dokuz ölçüm/round, toplam 27 warm sample. M1/8 GiB/macOS 26.1,
dosya başına 4096 byte. Önce/sonra checkout'ları başka remediation değişiklikleri de içerir.

| Dosya | Sık eşleşme: önce → sonra ms | Nadir eşleşme: önce → sonra ms | Eşleşme yok: önce → sonra ms |
| ---: | ---: | ---: | ---: |
| 200 | 14,960 → 15,500 (%3,6 daha yavaş) | 27,666 → 28,757 (%3,9 daha yavaş) | 27,757 → 28,469 (%2,6 daha yavaş) |
| 2.000 | 21,561 → 20,915 (%3,0 daha kısa) | 292,038 → 273,640 (%6,3 daha kısa) | 295,911 → 276,504 (%6,6 daha kısa) |
| 5.000 | 37,215 → 32,238 (%13,4 daha kısa) | 1737,027 → 731,120 (%57,9 daha kısa) | 1688,226 → 729,393 (%56,8 daha kısa) |

5.000-file rare baseline round medyanları 1926/1808/**743 ms**; candidate 797/731/720 ms.
Dolayısıyla pooled **2,38× oranını FFF'nin sağladığı kesin kazanç diye sunamayız**: baseline'ın
en hızlı turu candidate'a zaten yakın. 2.000 dosyada görülen %3–6,6 iyileşme de kontrollü
matcher-only deney olmadan tek başına FFF'ye atfedilemez. Bu ilk seri net FFF kazancını
izole etmiyordu.

Bağımsız full/indexed FFF 5.000 dosyada rare sorgusunu ~36,6 ms'de bitirdi; VSH candidate
~731,1 ms. Fakat bağımsız FFF snapshot/diff/policy/read-set/commit evidence işini yapmıyor,
aktif sanal overlay'i de aynı sözleşmeyle aramıyor. VSH'de yalnız `fff-grep` byte-slice
matching kullanıldı; full host index/watch/mmap eklenmedi. Güvenlik sınırı korunurken
indeksli ürünün bütün hız avantajını otomatik elde etmedik.

Kaynaklar: [performans dokümanı](../../docs/performance.md),
[baseline JSON](../../target/fff_search_benchmark_2026_10_05/results.json),
[candidate JSON](../../target/fff_search_benchmark_2026_10_05/after-integration.json),
[benchmark harness](../../target/fff_search_benchmark_2026_10_05/compare.py).
OS planının core/deployment ayrımı ve performans iddialarının sınırlandırılması
short-circuit planlama akışıyla yapıldı. Fetched belgeler yalnız teknik kanıt; scope
otoritesi kullanıcının erteleme kararıdır.
