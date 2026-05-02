# ECS

Regarding rationale about ECS - I've just pinpointed during the talk that the hydration of the entities is one of the most important reasons for  using the sparse-set ECS. But, was proposed another solution (if I correctly got it) - during hydration we are not just shuffling the components, we removing the entity completely, and moving it to the `world` data bank. But we still need to use sparse-set, since the game would have a lot of immersive-sim features.

# Two languages

## Where I agree with you

### The cost estimate was optimistic

From one point of you right - making a new language is hard. But

- I genuely love this and have a plenty of experience in that field
- I don't have bounds regarding the release time. The game must be bug-free, performant and pleasure to develop.

### The "don't throw out shik" argument has merit

Shik is indeed done, but right now it is a scripting language for shell automation scripts - dynamic, tree-walk intepretator. The language for the game must have similar syntax, but it would be a separate, byte-coded language, with static types for safety.

### CK3 does use one language

This is factually correct and I was imprecise. CK3's scripting system is a single unified language/format for events, decisions, character interactions, GUI, and game rules. It's not two separate languages.

BUT - **The CK3 analogy proves the opposite of what you thinks**

CK3 uses one language, yes. But look at what that language actually IS. It's not a general-purpose scripting language. It's a **declarative, domain-specific data format** with evaluated expressions:

```
# CK3 event definition — this is NOT procedural code
namespace = peasant_faction

peasant_faction.0001 = {
    type = country_event
    title = peasant_faction.0001.t
    desc = peasant_faction.0001.desc

    trigger = {
        is_ruler = yes
        primary_title.tier <= tier_county
        any_owned_county = {
            peasant_unrest >= 0.7
        }
        NOT = { has_trait = charitable }
        liege = {
            opinion = {
                target = root
                value < -10
            }
        }
    }

    weight_multiplier = {
        base = 1
        modifier = {
            add = 0.5
            NOT = { culture = liege.culture }
        }
    }

    option = {
        name = peasant_faction.0001.a
        start_war = {
            casus_belli = peasant_revolt
            target = root.primary_title
        }
    }
}
```

This is not Lua. This is not a Lisp. This is not a language you'd write a spell particle system in, or an HTN planner in, or a UI layout in. It's a structured data format with embedded predicate logic and scoped queries.

Now look at what your Glyph needs to do:

```lisp
;; Spell composition — procedural, higher-order, macro-driven
(defspell fireball
  :on-hit (fn [self target hit-pos]
    (match-components target
      [(WaterElement water)]
       (do (remove-component! target WaterElement)
           (spawn-area-effect hit-pos Steam {:radius 5.0}))
      [(Flammable flam)]
       (ignite! target {:intensity (* self.damage flam.susceptibility)})
      [_] 
       (apply-damage! target self.damage :type Fire))))

;; HTN plan decomposition — procedural with backtracking
(defplan investigate [noise-source]
  (sequential
    (move-to noise-source)
    (look-around {:radius 5.0 :duration 3.0})
    (branch
      (when found-evidence?
        (sequential
          (increase-suspicion 0.3)
          (plan search (current-area))))
      (otherwise
        (sequential
          (decrease-suspicion 0.2)
          (plan return-to-patrol))))))

;; UI layout — declarative but with reactive bindings
(defpanel spell-crafting
  (vertical
    (for [slot (player-spell-slots)]
      (spell-slot-widget slot 
        :on-drop #'add-component
        :highlight (valid-combination? slot (dragged-item))))))
```

These are **fundamentally different evaluation models**:

```
CK3-style rules:                    Glyph-style gameplay:
────────────────────                 ────────────────────────
Evaluate predicates on data          Execute sequential procedures
Query relationships                  Mutate entity state
Fire on slow ticks                   Run at 60fps
No control flow                      Closures, higher-order functions
No mutation during evaluation        Mutation is the whole point
Scoped variable resolution           Lexical scoping
Weighted random selection            Deterministic execution
Trigger → Effect (two phases)        Continuous execution
```

CK3's "one language" works because **CK3 doesn't have Glyph's problem domain.** CK3 has no real-time spell system, no physics-driven sandbox interactions, no HTN planning, no procedural NPC behavior authored in script. CK3's gameplay logic is in C++. The scripting layer ONLY handles the rule/content authoring.

Your game needs both. That's the fundamental difference from CK3.

### The "one language shaped like a Lisp can do both" argument

You say shik's reactive Lisp shape handles both `(when (and ruler? (< opinion -10)) ...)` and procedural gameplay. Let's actually test this claim by trying to express Chronicle-style rules in a Lisp:

```lisp
;; Attempt 1: CK3-style event as a shik function
(defevent peasant-revolt
  (when (and 
          (ruler? scope)
          (<= (rank scope) :baron)
          (>= (peasant-unrest (primary-territory scope)) 0.7)
          (not (has-trait? scope :charitable))
          (< (opinion scope (liege scope)) -10))
    (with-weight (+ 100 
                    (if (not= (culture scope) (culture (liege scope))) 50 0))
      (let [leader (spawn-character! {...})]
        (start-war! leader scope :peasant-revolt)
        (modify-opinion! scope (liege scope) -20)))))
```

This works syntactically. But now consider what has to happen at evaluation time:

1. The engine must iterate ALL living characters to find those matching the trigger
2. For each, it evaluates a chain of relational queries: `(liege scope)`, `(primary-territory scope)`, `(peasant-unrest territory)`, `(culture X)`
3. Each query touches the world database, not the ECS
4. This evaluation runs on the world thread, at monthly tick rate
5. The results are weighted, randomly sampled, and queued
6. Effects (`start-war!`, `modify-opinion!`) mutate the world database, not ECS components

Now consider what Glyph does when a spell hits something:

1. Pattern match on the target's ECS components (immediate, local)
2. Mutate ECS state (remove component, spawn entity, apply damage)
3. This runs on the main thread, at 60fps, during physics callbacks
4. It touches the ECS, not the world database
5. Execution is deterministic and sequential

**These are different runtimes, different data sources, different threading contexts, different tick rates, and different execution semantics.** You CAN jam both into one language, but then the language runtime needs to:

- Know whether it's running on the main thread or the world thread
- Know whether `(get-component entity X)` should query the ECS or the world database
- Handle the fact that `(liege scope)` is a relational query and `(get-component entity Position)` is a sparse-set lookup — completely different operations behind the same syntax
- Somehow make the type system understand that `opinion` returns `Int[-100..100]` in world-context but doesn't exist in gameplay-context
- Manage the fact that Chronicle-style evaluation needs to find ALL matching entities (set-returning query) while Glyph-style evaluation operates on specific known entities

The language becomes a leaky abstraction. The "one language" isn't really one language — it's two runtimes hiding behind one syntax, and the programmer must constantly know which runtime they're in.

### The "it's a scheduler concern, not a language concern" argument

This is the most sophisticated counter-argument, and it's almost right. The claim: just run shik scripts on the world thread at world-tick rate. The language is the same; only the scheduling differs.

The problem is that **the query model is different, not just the tick rate.**

Chronicle rules need to express:

```
"Find all characters who are rulers, whose rank is at most Baron,
whose primary territory has unrest above 0.7, whose liege's 
culture differs from their own."
```

This is a **set-returning relational query with joins across multiple tables.** It's the fundamental operation. Every single rule starts with this kind of query.

In a Lisp, you'd write this as:

```lisp
(filter characters
  (fn [c] (and (ruler? c)
               (<= (rank c) :baron)
               (>= (peasant-unrest (primary-territory c)) 0.7)
               (not= (culture c) (culture (liege c))))))
```

This works, but it's **O(n) with nested lookups per element.** The relational database could execute this as an indexed join in a fraction of the time if it knew the query structure at compile time. By expressing it as an opaque lambda, you've hidden the query plan from the optimizer.

Chronicle as a separate language can compile triggers into optimized query plans:

```
// Chronicle compiler output (pseudocode):
// 1. Scan rulers index (pre-built, O(rulers) not O(all_characters))
// 2. Filter by rank using indexed field
// 3. Join with territory table on primary_territory
// 4. Filter by peasant_unrest using indexed field
// 5. Join with vassalage table to find liege
// 6. Compare culture fields

QueryPlan {
    source: Index::Rulers,
    filters: [
        FieldFilter { field: "rank", op: Lte, value: Baron },
    ],
    joins: [
        Join { table: Territories, on: "primary_territory", 
               filter: FieldFilter { field: "peasant_unrest", op: Gte, value: 0.7 } },
        Join { table: Vassalage, on: "liege",
               filter: CrossFieldFilter { field: "culture", op: Neq, other: "scope.culture" } },
    ],
}
```

A general-purpose Lisp **cannot do this** without essentially embedding a query language inside it — at which point you have two languages anyway, just with worse ergonomics because the query syntax is constrained by the host language.

# Wrap up

What do you think of that? The language examples I showed you is not real languages, it is just a thought. Both languages must be:

- Convinient to write code for the domain it is made of
- Prototyping, experiments, code writing must be simple and fast
- Code for production must secure and stable, use static typing (and even refinment typing) for ensuring security, but make syntax clean. Types should secure the domain logic, and be designed specifically for the domain.
- Code must be performant for the domain.


If you agree, make a quick summary, and I will go the next step - talking about graphics style and scope for graphics in general.
