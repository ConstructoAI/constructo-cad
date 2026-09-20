/**
 * Relais `postMessage` entre la page de l'ERP Constructo et le moteur CAO.
 *
 * Le moteur est servi sous `/cad/` sur une origine SEPAREE de l'ERP
 * (`cad.constructoai.ca` contre `app.constructoai.ca`), et c'est toute sa
 * securite : 425 000 lignes de Rust tiers ne peuvent pas lire le `localStorage`
 * de l'ERP, donc pas atteindre le jeton `erp_token`. Ce fichier est la SEULE
 * porte percee dans ce mur. Tout ce qu'il laisse passer, il l'a juge.
 *
 * Il n'est PAS compile par trunk : il est copie tel quel dans le bundle
 * (`<link data-trunk rel="copy-file">` dans `web-app.html`) et charge par une
 * balise `<script>` ordinaire. Pas de module, pas de dependance, pas d'outil.
 *
 * ----------------------------------------------------------------------------
 * CE QUE LE MOTEUR EXPOSE VRAIMENT (verifie dans `src/app/control/mod.rs`)
 * ----------------------------------------------------------------------------
 * `ocs_control_submit(json)` NE REND PAS LA REPONSE. Il rend un BILLET
 * (`web-<ms>-<serie>`, `mod.rs:72`) et depose la requete dans une file. C'est la
 * boucle iced qui la draine, toutes les 50 ms (`view/mod.rs:2704`), puis range
 * le resultat sous ce billet. On le recupere par `ocs_control_take(billet)`
 * (`mod.rs:89`), qui rend `undefined` tant que rien n'est pret.
 * -> IL Y A DONC DEUX ATTENTES, ET ELLES NE SE RESSEMBLENT PAS :
 *   1. le billet n'est pas encore honore (le moteur n'a pas tourne) ;
 *   2. le billet est honore mais l'operation continue — statut `accepted` ou
 *      `running` (`mod.rs:463`, `mod.rs:724`). Il faut alors redemander par
 *      `{"op":"operation","request_id":...}`, qui est une REQUETE DE PLUS.
 * Aucune des deux ne se fait par une boucle serree : le moteur tourne dans CE
 * fil. Un `while` qui attend le moteur empeche le moteur de repondre — le gel
 * serait garanti, pas probable. On attend donc par `setTimeout`, ce qui rend la
 * main a la boucle de rendu entre chaque coup d'oeil.
 */
(function () {
  'use strict';

  var CANAL = 'constructo-cad';

  // Un seul plafond par requete, pose a son ARRIVEE : il couvre l'attente du
  // moteur, l'attente du billet et le suivi de l'operation. Un plafond par
  // etape se cumulerait en silence et ne bornerait rien.
  var DELAI_DEFAUT_MS = 120000;
  var DELAI_MIN_MS = 1000;
  var DELAI_MAX_MS = 600000;

  // Le moteur draine sa file toutes les 50 ms : regarder plus souvent ne rend
  // pas la reponse plus tot, ca ne fait que voler du temps au rendu. On part
  // court pour les operations instantanees, puis on s'espace.
  var PAS_MIN_MS = 25;
  var PAS_MAX_MS = 200;
  var PAS_MOTEUR_MS = 100;

  // Le wasm pese plus de 12 Mo. Sur un lien lent il arrive apres la page, et
  // une requete envoyee entre-temps doit ATTENDRE, pas disparaitre.
  var FILE_MAX = 1000;

  // Statuts NON FINAUX. Tout le reste est final, y compris `waiting_input`
  // (l'operation a rendu la main, elle attend une saisie du parent) et les
  // reponses de lecture qui n'ont pas de champ `status` du tout.
  var EN_COURS = { accepted: true, running: true };

  /**
   * Appartenance a une table, sans passer par la chaine de prototypes.
   * `EN_COURS['constructor']` vaut VRAI avec un `in` ou un acces direct, parce
   * qu'il remonte a `Object.prototype` : un statut nomme `constructor` ferait
   * boucler le suivi jusqu'au plafond, et un `op` nomme `toString` passerait
   * pour une lecture. Aucun des deux ne vient du moteur — mais une table qu'on
   * interroge doit repondre sur SON contenu, pas sur celui de sa grand-mere.
   */
  function dans(table, cle) {
    return typeof cle === 'string' && Object.prototype.hasOwnProperty.call(table, cle);
  }

  // --------------------------------------------------------------------------
  // 1. QUI A LE DROIT DE NOUS PARLER
  // --------------------------------------------------------------------------
  /**
   * L'origine de l'ERP ne peut pas etre ecrite en dur ici : l'ERP repond a
   * plusieurs noms (`app.constructoai.ca`, `erp.constructoai.ca`, l'hote Render,
   * un `localhost` en developpement), et un fork qui les recopierait divergerait
   * du jour ou l'un change.
   *
   * CE QU'UN ATTAQUANT CONTROLE, ET CE QU'IL NE CONTROLE PAS.
   * Il controle l'URL qu'il met dans SON iframe — donc un parametre d'URL est
   * une DECLARATION, jamais une preuve. Il ne controle ni `event.origin` (le
   * navigateur l'estampille), ni `location.ancestorOrigins` (le navigateur la
   * remplit), ni les en-tetes que notre serveur envoie.
   *
   * D'ou l'ordre suivant :
   *
   * a) `location.ancestorOrigins[0]` quand il existe (Chromium, WebKit) : c'est
   *    le navigateur qui le dit, il ne se falsifie pas. On l'utilise comme
   *    VERITE, et si un parametre le contredit on refuse TOUT — quelqu'un ment.
   *
   * b) A defaut (Firefox ne l'implemente pas), le parametre `?parent=`. Et la
   *    il faut etre franc : ce parametre ne prouve rien PAR LUI-MEME. Ce qui
   *    empeche `evil.com` de se declarer parent, c'est que le navigateur lui
   *    refuse deja le cadre : nos reponses portent
   *    `Content-Security-Policy: frame-ancestors <origines de l'ERP>`
   *    (`ERP_REACT/backend/cad_service.py`, `entetes_communs`). Le parametre
   *    designe LAQUELLE des origines autorisees nous parle ; l'en-tete decide
   *    qui a le droit d'etre la. Si cet en-tete disparaissait du service, ce
   *    fichier ne serait plus une frontiere sur Firefox.
   *
   * c) Mentir dans le parametre ne rapporte rien, et c'est voulu : declarer
   *    `?parent=https://app.constructoai.ca` depuis `evil.com` fait refuser les
   *    messages d'evil (`event.origin` ne correspond pas) ET fait partir nos
   *    reponses vers `app.constructoai.ca`, ou le navigateur les jette faute de
   *    destinataire. Le menteur se coupe la parole lui-meme.
   *
   * Un jeton passe dans l'URL serait inutile ici : celui qui ecrit l'URL ecrit
   * le jeton. Seul un secret que le CADRE peut verifier aupres de son propre
   * serveur vaudrait mieux — c'est la version forte, elle demande un point
   * d'appel cote ERP et n'existe pas encore.
   */
  function origineNormalisee(valeur) {
    if (typeof valeur !== 'string' || valeur === '' || valeur === 'null') return '';
    var url;
    try {
      url = new URL(valeur);
    } catch (erreur) {
      return '';
    }
    var hote = url.hostname;
    var local = hote === 'localhost' || hote === '127.0.0.1' || hote === '[::1]';
    // `https:` exige, sauf en local : une origine `http:` publique ne protege
    // rien de ce qu'on lui confie, et `data:`/`file:`/un bac a sable rendent
    // l'origine opaque (`null`), qu'aucune comparaison ne doit accepter.
    if (url.protocol !== 'https:' && !(url.protocol === 'http:' && local)) return '';
    return url.origin;
  }

  function resoudreParent() {
    if (window.parent === window) {
      return { origine: '', refus: 'hors-cadre (la page n’est pas encadree)' };
    }
    var declaree = '';
    try {
      declaree = origineNormalisee(new URL(window.location.href).searchParams.get('parent') || '');
    } catch (erreur) {
      declaree = '';
    }
    var ancetres = null;
    try {
      ancetres = window.location.ancestorOrigins || null;
    } catch (erreur) {
      ancetres = null;
    }
    if (ancetres && ancetres.length > 0) {
      var vue = origineNormalisee(ancetres[0]);
      if (!vue) {
        return { origine: '', refus: 'parent a origine opaque ou non https' };
      }
      if (declaree && declaree !== vue) {
        // Le parametre contredit le navigateur. On ne choisit pas le moins
        // faux : on ferme.
        return { origine: '', refus: 'parametre parent=' + declaree + ' contredit par le navigateur (' + vue + ')' };
      }
      return { origine: vue, source: 'ancestorOrigins' };
    }
    if (declaree) {
      return { origine: declaree, source: 'parametre parent= + frame-ancestors' };
    }
    return { origine: '', refus: 'aucun parametre parent= et ancestorOrigins indisponible' };
  }

  var parent = resoudreParent();
  var ORIGINE = parent.origine;
  if (!ORIGINE) {
    // On se tait DANS LES DEUX SENS : sans destinataire sur, il n'y a personne
    // a prevenir, et repondre a l'inconnu lui apprendrait qu'il a touche
    // quelque chose. La console de la page, elle, le dit.
    console.warn('[cad] relais ferme : ' + parent.refus);
  } else {
    console.info('[cad] relais ouvert vers ' + ORIGINE + ' (' + parent.source + ')');
  }

  function envoyer(charge) {
    if (!ORIGINE) return;
    // JAMAIS `'*'` : ce serait rendre la reponse lisible par n'importe quel
    // parent, donc annuler la separation d'origine par l'autre bout.
    try {
      window.parent.postMessage(charge, ORIGINE);
    } catch (erreur) {
      console.warn('[cad] reponse non remise :', erreur);
    }
  }

  function envoyerErreur(id, code, message) {
    envoyer({ canal: CANAL, type: 'erreur', id: id === undefined ? null : id, code: code, message: message });
  }

  // --------------------------------------------------------------------------
  // 2. LE MOTEUR EST-IL LA
  // --------------------------------------------------------------------------
  /**
   * trunk publie les exports wasm sur `window.wasmBindings` une fois le module
   * initialise ; certaines configurations les posent directement sur `window`.
   * On regarde les deux, et on juge sur la PRESENCE DES DEUX FONCTIONS, jamais
   * sur un drapeau ou un evenement : un drapeau peut precede les exports.
   * (trunk emet aussi `TrunkApplicationStarted` ; je ne m'y appuie pas, n'ayant
   * pas pu verifier que la version installee en CI l'emet encore.)
   */
  function exportsMoteur() {
    var candidats = [window.wasmBindings, window];
    for (var i = 0; i < candidats.length; i++) {
      var c = candidats[i];
      if (c && typeof c.ocs_control_submit === 'function' && typeof c.ocs_control_take === 'function') {
        return c;
      }
    }
    return null;
  }

  function attendreMoteur(echeance) {
    return new Promise(function (resoudre, rejeter) {
      var immediat = exportsMoteur();
      if (immediat) return resoudre(immediat);
      var coup = function () {
        var m = exportsMoteur();
        if (m) return resoudre(m);
        if (Date.now() >= echeance) {
          return rejeter({ code: 'moteur_indisponible', message: 'Le moteur CAO n’a pas fini de charger dans le delai imparti' });
        }
        setTimeout(coup, PAS_MOTEUR_MS);
      };
      setTimeout(coup, PAS_MOTEUR_MS);
    });
  }

  // --------------------------------------------------------------------------
  // 3. UNE OPERATION, DE BOUT EN BOUT
  // --------------------------------------------------------------------------
  function deposer(moteur, operation) {
    var billet;
    try {
      billet = moteur.ocs_control_submit(JSON.stringify(operation));
    } catch (erreur) {
      throw { code: 'relais_interne', message: 'ocs_control_submit a leve : ' + (erreur && erreur.message ? erreur.message : String(erreur)) };
    }
    if (typeof billet !== 'string' || billet === '') {
      throw { code: 'relais_interne', message: 'ocs_control_submit n’a pas rendu de billet' };
    }
    return billet;
  }

  function retirer(moteur, billet) {
    var brut;
    try {
      brut = moteur.ocs_control_take(billet);
    } catch (erreur) {
      throw { code: 'relais_interne', message: 'ocs_control_take a leve : ' + (erreur && erreur.message ? erreur.message : String(erreur)) };
    }
    // `Option<String>` du Rust arrive ici en `undefined` tant que la boucle
    // iced n'a pas draine la file. Ce n'est pas une erreur, c'est « pas encore ».
    if (brut === undefined || brut === null) return null;
    try {
      return JSON.parse(brut);
    } catch (erreur) {
      throw { code: 'relais_interne', message: 'reponse du moteur illisible : ' + brut };
    }
  }

  /** Attend le resultat d'un billet sans bloquer : un `setTimeout` qui s'espace. */
  function attendreBillet(moteur, billet, echeance) {
    return new Promise(function (resoudre, rejeter) {
      var pas = PAS_MIN_MS;
      var coup = function () {
        var reponse;
        try {
          reponse = retirer(moteur, billet);
        } catch (erreur) {
          return rejeter(erreur);
        }
        if (reponse !== null) return resoudre(reponse);
        if (Date.now() >= echeance) {
          return rejeter({ code: 'delai_depasse', message: 'Le moteur n’a pas honore le billet ' + billet + ' dans le delai imparti' });
        }
        pas = Math.min(Math.round(pas * 1.5), PAS_MAX_MS);
        setTimeout(coup, pas);
      };
      setTimeout(coup, PAS_MIN_MS);
    });
  }

  function executer(operation, echeance) {
    var moteur;
    return attendreMoteur(echeance)
      .then(function (m) {
        moteur = m;
        return attendreBillet(moteur, deposer(moteur, operation), echeance);
      })
      .then(function (reponse) {
        // Statut non final : l'operation tourne toujours. On la SUIT par une
        // requete de lecture, qui ne consomme pas de `request_id` et n'est pas
        // refusee par le garde « busy » (`op:"operation"` est une lecture).
        var identifiant = (reponse && reponse.request_id) || operation.request_id || '';
        if (!reponse || !dans(EN_COURS, reponse.status) || !identifiant) return reponse;

        var suivre = function () {
          if (Date.now() >= echeance) {
            return Promise.reject({
              code: 'delai_depasse',
              message: 'Operation ' + identifiant + ' toujours en cours (' + reponse.status + ') apres le delai imparti'
            });
          }
          return new Promise(function (resoudre) { setTimeout(resoudre, PAS_MOTEUR_MS); })
            .then(function () {
              return attendreBillet(
                moteur,
                deposer(moteur, { protocol: 1, op: 'operation', request_id: identifiant }),
                echeance
              );
            })
            .then(function (etat) {
              if (etat && dans(EN_COURS, etat.status)) return suivre();
              return etat;
            });
        };
        return suivre();
      });
  }

  // --------------------------------------------------------------------------
  // 4. LA FILE — le moteur ne tient QU'UNE operation a la fois
  // --------------------------------------------------------------------------
  /**
   * `control_request` refuse toute seconde operation tant que la premiere n'est
   * pas reglee (`failure("busy", "Wait for the running operation")`,
   * `mod.rs:584`). Un parent qui envoie les 200 commandes d'un script d'un coup
   * recevrait donc 199 refus. On serialise ici : le parent envoie quand il veut,
   * le relais fait passer un a la fois, dans l'ordre d'arrivee.
   */
  var file = Promise.resolve();
  var enAttente = 0;
  var generation = 0;

  function enfiler(id, tache) {
    if (enAttente >= FILE_MAX) {
      envoyerErreur(id, 'file_pleine', 'Plus de ' + FILE_MAX + ' operations en attente dans le relais');
      return;
    }
    enAttente++;
    var marque = generation;
    // `file` NE DOIT JAMAIS ETRE UNE PROMESSE REJETEE, et c'est plus subtil
    // qu'il n'y parait. Avec un `file.then(succes, echec)`, un rejet du maillon
    // PRECEDENT declenche le gestionnaire d'echec du SUIVANT : sa tache n'est
    // jamais lancee, et sa requete n'obtient jamais de reponse — perdue en
    // silence, exactement ce qu'on veut interdire. On rattache donc le rattrapage
    // a CE maillon-ci (`suite`), et on ne remet dans la file qu'un etat resolu.
    var suite = file.then(function () {
      enAttente--;
      // Un `abandon` du parent ne peut pas defaire ce que le moteur a deja
      // fait, mais il ne doit pas non plus laisser une requete sans reponse.
      if (marque !== generation) {
        envoyerErreur(id, 'abandonne', 'Operation retiree de la file a la demande du parent');
        return;
      }
      return tache();
    });
    file = suite.catch(function () {});
    suite.catch(function (erreur) {
      envoyerErreur(id, 'relais_interne', (erreur && erreur.message) || String(erreur));
    });
  }

  // --------------------------------------------------------------------------
  // 5. LE GUICHET
  // --------------------------------------------------------------------------
  var compteur = 0;

  function identifiantRelais() {
    compteur++;
    return 'erp-' + Date.now().toString(36) + '-' + compteur;
  }

  // Lectures : le moteur ne leur demande pas de `request_id` et ne les bloque
  // pas quand l'automatisation est arretee (`mod.rs:415-432`).
  var LECTURES = {
    state: true, hello: true, operation: true, events: true, commands: true,
    properties: true, measure: true, query: true, records: true, record_schema: true,
    capabilities: true, entities: true, layers: true, header: true, history: true
  };

  function preparer(operation) {
    var prete = {};
    for (var cle in operation) {
      if (Object.prototype.hasOwnProperty.call(operation, cle)) prete[cle] = operation[cle];
    }
    if (prete.protocol === undefined) prete.protocol = 1;
    if (!dans(LECTURES, prete.op) && (typeof prete.request_id !== 'string' || prete.request_id === '')) {
      // LE MOTEUR EXIGE UN `request_id` POUR TOUTE ECRITURE, et il s'en sert
      // comme CLE D'IDEMPOTENCE : rejouer le meme identifiant avec la meme
      // charge rend le resultat memorise au lieu de redessiner (`mod.rs:562`).
      // On en fabrique un quand le parent n'en donne pas, sinon rien ne passe —
      // mais on le lui RENVOIE dans la reponse, parce qu'un reessai qui en
      // regenererait un autre dessinerait la figure DEUX FOIS.
      prete.request_id = identifiantRelais();
    }
    return prete;
  }

  function traiter(message) {
    var id = message.id === undefined ? null : message.id;
    var operation = message.operation;
    if (!operation || typeof operation !== 'object' || typeof operation.op !== 'string' || operation.op === '') {
      envoyerErreur(id, 'requete_invalide', 'Champ `operation` absent, non objet, ou sans `op`');
      return;
    }
    var delai = Number(message.delaiMs);
    if (!isFinite(delai) || delai <= 0) delai = DELAI_DEFAUT_MS;
    delai = Math.min(Math.max(delai, DELAI_MIN_MS), DELAI_MAX_MS);
    var echeance = Date.now() + delai;
    var prete = preparer(operation);

    enfiler(id, function () {
      return executer(prete, echeance).then(
        function (reponse) {
          envoyer({
            canal: CANAL,
            type: 'reponse',
            id: id,
            requestId: prete.request_id || null,
            reponse: reponse
          });
        },
        function (erreur) {
          envoyerErreur(
            id,
            (erreur && erreur.code) || 'relais_interne',
            (erreur && erreur.message) || String(erreur)
          );
        }
      );
    });
  }

  function etatRelais() {
    return {
      canal: CANAL,
      type: 'pret',
      moteurCharge: exportsMoteur() !== null,
      origine: ORIGINE,
      enAttente: enAttente
    };
  }

  window.addEventListener('message', function (evenement) {
    var donnees = evenement.data;
    // Le silence est la reponse par defaut : cette page recoit aussi les
    // messages d'autres outils (rechargement a chaud, extensions). On ne
    // journalise pas ce qui ne nous est pas adresse, sinon la console devient
    // illisible et on n'y voit plus les vrais refus.
    if (!donnees || typeof donnees !== 'object' || donnees.canal !== CANAL) return;
    if (!ORIGINE) return;
    // Les deux gardes qu'un attaquant ne peut pas contourner depuis sa page :
    // l'origine estampillee par le navigateur, et l'identite de la fenetre.
    if (evenement.origin !== ORIGINE || evenement.source !== window.parent) {
      console.warn('[cad] message refuse : origine ' + evenement.origin + ' au lieu de ' + ORIGINE);
      return;
    }
    switch (donnees.type) {
      case 'requete':
        traiter(donnees);
        break;
      case 'sonde':
        // Repond sans toucher au moteur : sert au parent qui aurait installe
        // son ecouteur apres l'annonce `pret`.
        envoyer(etatRelais());
        break;
      case 'abandon':
        // Vide ce que le relais n'a pas encore soumis. Ce qui est deja parti
        // dans le moteur suit son cours — le dire, plutot que le laisser croire.
        generation++;
        envoyer({ canal: CANAL, type: 'abandon', id: donnees.id === undefined ? null : donnees.id, retirees: enAttente });
        break;
      default:
        envoyerErreur(donnees.id, 'requete_invalide', 'Type de message inconnu : ' + donnees.type);
    }
  });

  // Annonce unique quand le moteur devient utilisable. Le parent peut aussi la
  // demander par `sonde` : une annonce ratee ne doit pas condamner la session.
  if (ORIGINE) {
    attendreMoteur(Date.now() + DELAI_MAX_MS).then(
      function () { envoyer(etatRelais()); },
      function () {
        envoyerErreur(null, 'moteur_indisponible', 'Le moteur CAO n’a pas demarre');
      }
    );
  }
})();
