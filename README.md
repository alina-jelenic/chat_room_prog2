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
│   └── integration.rs            # integracijski testi (HTTP + WebSocket)
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

## Zagon projekta

Za uporabo projekta, se je najprej treba odločiti, kateri računalnik bo deloval kot strežnik (Za ostale računalnike oz. uporabnike je po uspostavitvi strežnika potreben le dostop do interneta). Potem na tem računalniku izvedemo naslednje korake, ko že imamo naložen Rust, prenesen GitHub repozitorij in vse potrebne pakete.

1. Kopiraj .env.example v .env in po potrebi prilagodi vrednosti (predvsem JWT_SECRET, ki mora biti dolg vsaj 32 znakov).
2. Zaženi aplikacijo:
```sh
   cargo run
```
   Ob zagonu se samodejno izvedejo vse manjkajoče migracije in ustvari
   soba `#general`, če še ne obstaja.

3. Odloči se, kako bodo odjemalci dostopali do strežnika, in ustrezno
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

   2. Zaženi aplikacijo lokalno (`cargo run`, glej korak 2 zgoraj).
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
- **Upravljanje članstva**: pridružitev, zapustitev sobe in brisanje sobe
  (samo lastnik), z ustreznimi omejitvami dostopa do zasebnih sob.
- **Sporočila v realnem času** prek WebSocket povezave in HTMX (`ws-send`,
  `hx-swap-oob`) brez ročnega pisanja JavaScripta na frontendu. 
- **Zgodovina sporočil s straničenjem**: sporočila se nalagajo po straneh
  (50 na stran), starejša sporočila se naložijo na zahtevo. 
- **Odgovori na sporočila (threads)**: zraven vsakega sporočila je gumb
  "Odgovori", ki  prikaže banner nad vnosnim poljem s citatom sporočila, 
  na katerega odgovarjaš. Poslan odgovor v pogovoru prikaže kratek citat 
  izvirnega sporočila in avtorja.
- **Migracije baze**, ki ohranjajo obstoječe podatke pri nadgradnji sheme
  (npr. dodajanje stolpca za geslo ali povezovanje sporočil s sobami na
  starejših, že napolnjenih bazah).


## Testiranje

Integracijski testi (`tests/integration_tests.rs`) pokrivajo avtentikacijo,
upravljanje sob in članstva, straničenje in iskanje po sporočilih, reakcije
na sporočila, odgovore na sporočila (threads) ter WebSocket komunikacijo
(vključno z zavračanjem nepooblaščenih povezav, omejevanjem hitrosti
pošiljanja in obveščanjem uporabnikov ob izbrisu sobe) in pokrivajo tudi
robne primere. Zaženemo jih z ukazom:
 
```sh
cargo test --all
```
 



