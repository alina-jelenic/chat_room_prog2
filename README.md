# Projektna naloga pri predmetu Programiranje 2

## Vsebina projekta

V tem projektu sva ustvarila chat-room v realnem času. Z orodji, ki jih ponuja Rust, lahko tako hkrati vodiva pogovore z več uporabniki z maksimalno (ne)stabilnostjo in (ne)varnostjo.

## Zakaj sva se odločila za chat-room?

Ta projekt sva izbrala iz več razlogov, saj sva po analizi več možnosti ugotovila, da je prilagojen delu v Rustu, tudi za nove uporabnike, in da bo in tudi je na najboljši način prispeval k razvoju naših programskih veščin v tem jeziku. Bolj natančno, delo na tem projektu nama je pomagalo napredovati natanko na tistih področjih, ki jih potrebujemo.

Z uporabo asinhronega modela sva ustvarila sistem, ki temelji na arhitekturi odjemalec–strežnik, kjer se več odjemalcev poveže na strežnik, ta pa skrbi za posredovanje sporočil med njimi.

## Struktura projekta

```
├── Cargo.toml              # konfiguracija glavnega paketa in odvisnosti
├── README.md                # opis projekta
├── .env.example              # predloga za okoljske spremenljivke
├── src/
│   ├── main.rs                # vstopna točka: povezava z bazo, migracije, zagon strežnika
│   ├── lib.rs                 # javni moduli knjižnice (controller, entities)
│   ├── controller/
│   │   ├── mod.rs               # deklaracija podmodulov
│   │   ├── auth.rs              # JWT, seje, avtentikacijski middleware
│   │   ├── forms.rs             # obdelava prijave in registracije
│   │   ├── rooms.rs             # logika sob: ustvarjanje, pridružitev, dostop
│   │   ├── rooms/
│   │   │   ├── messages.rs        # zgodovina, iskanje, brisanje sporočil
│   │   │   ├── reactions.rs       # emoji reakcije na sporočila
│   │   │   ├── reply.rs           # nastavljanje/čiščenje odgovora (threads) 
│   │   │   └── views.rs           # skupni HTML delci (soba, člani, obvestila)
│   │   ├── tipi.rs              # deljeno stanje strežnika (SharedState)
│   │   ├── util.rs              # skupni modul za html_escape
│   │   └── web.rs               # axum router, WebSocket handler
│   └── entities/                # SeaORM modeli (client, soba, message, room_member)
├── migration/                 # ločen paket z migracijami (sea-orm-migration)
│   ├── src/
│   │   ├── main.rs               # CLI za poganjanje migracij
│   │   ├── lib.rs                 # seznam vseh migracij
│   │   └── m*.rs                   # posamezne migracijske datoteke
│   └── README.md                 # navodila za uporabo migracijskega CLI-ja
├── static/                    # statične HTML/CSS datoteke frontenda
│   ├── index.html                # glavni vmesnik klepetalnice
│   └── authorisation.html         # prijava in registracija
├── tests/
│   ├── integration_tests.rs          # vstopna datoteka, ki poveže vse module integracijskih testov
│   ├── common/
│   │   └── mod.rs                    # skupne testne funkcije: priprava baze, aplikacije, sej in WebSocket povezav
│   ├── authorisation/
│   │   ├── mod.rs                    # deklaracija testnih modulov za avtentikacijo
│   │   ├── jwt.rs                    # testi podpisa, veljavnosti, poteka in skrivnosti JWT
│   │   ├── registration.rs           # testi registracije, validacije in unikatnosti uporabniških imen
│   │   └── sessions.rs               # testi prijave, piškotka seje, zaščitenih poti, /me in odjave
│   ├── frontend/
│   │   ├── mod.rs                    # deklaracija testnega modula za frontend
│   │   └── frontend_test.rs          # testi glavnega uporabniškega toka in prikaza stanja WebSocket povezave
│   ├── messages/
│   │   ├── mod.rs                    # deklaracija testnih modulov za sporočila
│   │   ├── deletion.rs               # testi avtorizacije, brisanja in realnočasovnega obveščanja
│   │   ├── history.rs                # testi straničenja, vrstnega reda, HTML escaping-a in obstojnosti sporočil
│   │   ├── reactions.rs              # testi dodajanja, odstranjevanja in oddajanja reakcij
│   │   ├── reply.rs                  # testi odgovorov s citatom in preverjanja izvirne sobe
│   │   └── search.rs                 # testi iskanja, omejevanja na sobo, dostopa in HTML escaping-a
│   ├── migrations/
│   │   ├── mod.rs                    # deklaracija testnega modula za migracije
│   │   └── migrations_test.rs        # testi idempotentnosti migracij in ohranjanja podatkov stare baze
│   ├── rooms/
│   │   ├── mod.rs                    # deklaracija testnih modulov za sobe
│   │   ├── creation.rs               # testi validacije, podvojenih imen in sočasnega ustvarjanja sob
│   │   ├── deletion.rs               # testi brisanja sobe, obveščanja uporabnikov in blokiranja nadaljnjih sporočil
│   │   ├── kick.rs                   # testi lastnikovega izključevanja članov in zapiranja njihovih povezav
│   │   └── membership.rs             # testi pridružitve, zapustitve, članstva in omejevanja dostopa
│   └── websocket/
│       ├── mod.rs                    # deklaracija testnih modulov za WebSocket
│       ├── membership.rs             # testi povezav brez članstva ter zapiranja aktivnih in pasivnih povezav
│       ├── messaging.rs              # testi shranjevanja in oddajanja sporočil več povezanim uporabnikom
│       ├── rate_limit.rs             # testi skupne omejitve hitrosti med več povezavami uporabnika
│       └── sessions.rs               # testi zavračanja WebSocket povezav brez veljavne seje
└── .github/workflows/ci.yml    # CI: fmt, clippy, testi
```
## Uporabljeni paketi
 
- **axum** — spletni strežnik in usmerjanje (vključno z WebSocket nadgradnjo)
- **SeaORM** + **sea-orm-migration** — ORM in upravljanje migracij (SQLite)
- **tokio** — asinhrono izvajanje
- **jsonwebtoken** — seje prek JWT
- **argon2** — varno zgoščevanje gesel
- **HTMX** (`htmx-ext-ws`) — dinamičen frontend brez pisanja JavaScripta;
  vključno z reakcijami na sporočila in odgovori (threads), ki so v
  celoti implementirani prek `hx-swap-oob` fragmentov s strežnika

**Opomba:** V kodi je uporabljeno malo JavaScripta za indikator povezave, saj tega ni mogoče napisati s HTMX. 

## Zagon projekta

Za uporabo projekta, se je najprej treba odločiti, kateri računalnik bo deloval kot strežnik (Za ostale računalnike oz. uporabnike je po uspostavitvi strežnika potreben le dostop do interneta). Potem na tem računalniku izvedemo naslednje korake, ko že imamo naložen Rust, prenesen GitHub repozitorij in vse potrebne pakete.

1. Kopiraj .env.example v .env.
2. V datoteki `.env` obvezno zamenjaj vrednost `JWT_SECRET=CHANGE_ME`
   z naključno skrivnostjo, dolgo vsaj 32 znakov. Datoteke `.env` ne
   dodajaj v Git, saj vsebuje lokalno skrivnost.

   Primer:

   ```env
   JWT_SECRET=tukaj-vstavi-dolgo-nakljucno-skrivnost-z-vsaj-32-znaki
3. Zaženi aplikacijo:
```sh
   cargo run
```
   Ob zagonu se samodejno izvedejo vse manjkajoče migracije in ustvari
   soba `#general`, če še ne obstaja.

4. Odloči se, kako bodo odjemalci dostopali do strežnika, in ustrezno
   nastavi `SERVER_ADDR` v `.env`.

    ### a) Samo na istem računalniku

   Privzeta nastavitev že deluje brez sprememb (`SERVER_ADDR=127.0.0.1:3000`).
   Aplikacijo odpreš v brskalniku na istem računalniku na naslovu: http://127.0.0.1:3000

    ### b) Več naprav v istem omrežju (isti WiFi)

   Strežnik mora poslušati na vseh omrežnih vmesnikih, ne samo na
   `127.0.0.1`. V `.env` nastavi: `SERVER_ADDR=0.0.0.0:3000`.
   Nato ugotovi lokalni IP naslov računalnika, na katerem teče strežnik:

   - Windows: `ipconfig` (poišči "IPv4 Address")
   - macOS / Linux: `ifconfig` ali `ip addr` (ali `hostname -I`)

   Drugi uporabniki v istem omrežju nato v brskalniku odprejo: http://<IP-naslov-strežnika>:3000
   Če se ne morejo povezati, preveri, ali požarni zid (firewall) na
   računalniku s strežnikom blokira vrata 3000 (npr. na Windows: Windows
   Defender Firewall → Dovoljene aplikacije; na Linuxu z `ufw`:
   `sudo ufw allow 3000`).

    ### c) Naprave v različnih omrežjih (dostop prek interneta)

   To zahteva, da je strežnik dosegljiv od zunaj tvojega domačega omrežja.
   Najenostavnejša pot je Cloudflare Quick Tunnel — brezplačna storitev, ki
   ne zahteva registracije računa ne lastne domene.

    1. Namesti orodje `cloudflared`:
      - **Windows:** `winget install Cloudflare.cloudflared`
      - **macOS:** `brew install cloudflare/cloudflare/cloudflared`
      - **Linux (Debian/Ubuntu):**
      ```sh
        curl -L https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb -o cloudflared.deb
        sudo dpkg -i cloudflared.deb
      ```

      Za druge distribucije glej [uradno stran za prenos](https://developers.cloudflare.com/tunnel/downloads/).

   2. Zaženi aplikacijo lokalno (`cargo run`, glej korak 3 zgoraj).
   3. V ločenem terminalu zaženi:
    ```sh
      cloudflared tunnel --url http://localhost:3000
    ```
   4. V izpisu poišči vrstico z naslovom, ki se konča na
      `.trycloudflare.com` (npr. `https://xxxx-xx-xx.trycloudflare.com`).
      Ta naslov deluje takoj in ga lahko deliš s komerkoli, ne glede na to,
      v katerem omrežju je — povezava ostane aktivna, dokler v terminalu
      teče `cloudflared` (`Ctrl+C` jo prekine).

   > **Opomba:** Quick Tunnel je namenjen priložnostnemu deljenju in
   > testiranju, ne trajni uporabi — naslov se ob vsakem zagonu spremeni, deluje dokler je odprt terminal
   > in ni namenjen produkcijski postavitvi.

## Glavne funkcionalnosti
 
- **Registracija in prijava** z uporabniškim imenom in geslom; gesla so
  zgoščena z argon2, seja pa se hrani v HttpOnly piškotku kot JWT žeton.
- **Klepetalne sobe**: soba `#general` je na voljo vsem, dodatne sobe pa
  lahko uporabniki ustvarijo (postanejo njihov lastnik) ali se jim
  pridružijo prek numeričnega ID-ja sobe.
- **Upravljanje članstva**: pridružitev in zapustitev sobe ter pregled in
  izključevanje članov s strani lastnika. Sobo lahko izbriše samo njen lastnik.
- **Sporočila v realnem času** prek WebSocket povezave in HTMX (`ws-send`,
  `hx-swap-oob`) brez ročnega pisanja JavaScripta na frontendu. 
- **Zgodovina sporočil s straničenjem**: sporočila se nalagajo po straneh
  (50 na stran), starejša sporočila se naložijo na zahtevo. 
- **Odgovori na sporočila **: zraven vsakega sporočila je gumb
  "Odgovori", ki  da možnost, da odgovoriš na določeno sporočilo. Pri tem se
  nad tem sporočilom prikaže trak, tako da se ve kateremu sporočilu si dal
  odgovor.
- **Migracije baze**, ki ohranjajo obstoječe podatke pri nadgradnji sheme
  (npr. dodajanje stolpca za geslo ali povezovanje sporočil s sobami na
  starejših, že napolnjenih bazah).
- **Iskanje po zgodovini** z omejitvijo rezultatov na trenutno sobo.
- **Brisanje lastnih sporočil**, pri katerem se sprememba v realnem času
  prikaže vsem povezanim uporabnikom.
- **Reakcije na sporočila**, ki jih lahko uporabniki dodajo ali odstranijo.
- **Omejevanje hitrosti pošiljanja**, skupno vsem povezavam istega uporabnika.

## Testiranje

Integracijski testi, povezani prek `tests/integration_tests.rs`, pokrivajo:

- registracijo, prijavo, JWT in sejne piškotke;
- ustvarjanje, članstvo, zapustitev in brisanje sob;
- straničenje, iskanje, odgovore, reakcije in brisanje sporočil;
- WebSocket komunikacijo več uporabnikov;
- zavračanje nepooblaščenih povezav;
- omejevanje hitrosti pošiljanja;
- zapiranje povezav po zapustitvi, izključitvi ali izbrisu sobe;
- migracije in ohranjanje podatkov stare baze.

Vse teste zaženemo z:

```sh
cargo test --workspace --all-targets --locked
```