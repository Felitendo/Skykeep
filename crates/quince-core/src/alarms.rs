//! Reminder alarms: when each one rings, and which have rung already.
//!
//! Apple keeps an alert as two records — an `Alarm` on the reminder and the `AlarmTrigger`
//! it names, which holds the moment — and both are stored as they come, joined only when
//! asking what is due. What has been shown is remembered by alarm and moment, which is what
//! makes an alert ring exactly once: across restarts, across a sleep, and across a pass that
//! brings the same alarm back unchanged.

use crate::{Library, Result};
use rusqlite::params;
use serde::Serialize;

/// An `Alarm` record: which reminder it is for and which trigger holds its moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alarm {
    pub id: String,
    pub reminder_guid: String,
    pub trigger_id: String,
    /// When a device last showed it and someone saw it (`AcknowledgedDate`), in seconds.
    pub acknowledged_at: Option<i64>,
}

/// An `AlarmTrigger` record, its wall-clock moment already resolved to an instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmTrigger {
    pub id: String,
    pub fires_at: i64,
}

/// An alert that is due, with what it needs to say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DueAlarm {
    /// The alarm's id, or `snooze:<reminder>` for one snoozed here.
    pub key: String,
    pub reminder_guid: String,
    pub list_guid: String,
    pub title: String,
    pub notes: Option<String>,
    pub list_title: Option<String>,
    pub fires_at: i64,
}

/// Every alert that may still ring: Apple's alarms on reminders not yet done and not already
/// seen on another device, and this device's snoozes. Not yet filtered for what has rung.
///
/// A repeating reminder's trigger keeps the date of an occurrence already past: the phone
/// moves the reminder's due date on and leaves the trigger where it was (measured — every
/// one-off alarm on the account sits exactly on its due date, and the repeating ones that
/// differ all lag behind it). So a repeating reminder rings at its current due date, and
/// each occurrence, being a new moment, rings once.
const PENDING: &str = "
    SELECT alarm_id AS key, reminder_guid, fires_at FROM (
        SELECT a.alarm_id, a.reminder_guid, a.acknowledged_at,
               CASE WHEN r.recur_frequency IS NOT NULL AND r.due_at > t.fires_at
                    THEN r.due_at ELSE t.fires_at END AS fires_at
          FROM reminder_alarms a
          JOIN alarm_triggers t ON t.trigger_id = a.trigger_id
          JOIN reminders r ON r.guid = a.reminder_guid)
     WHERE acknowledged_at IS NULL OR acknowledged_at < fires_at
    UNION ALL
    SELECT 'snooze:' || s.reminder_guid, s.reminder_guid, s.fires_at
      FROM alarm_snoozes s";

impl Library {
    /// Store the alarms and triggers a pass brought back.
    ///
    /// A whole pass replaces what is stored; an incremental one lays itself over it, less
    /// the records named in `gone` (`Alarm/<id>` or `AlarmTrigger/<id>`, as tombstones are).
    pub fn put_alarms(
        &mut self,
        alarms: &[Alarm],
        triggers: &[AlarmTrigger],
        gone: &[String],
        whole: bool,
    ) -> Result<()> {
        let tx = self.conn_mut().savepoint()?;
        if whole {
            tx.execute("DELETE FROM reminder_alarms", [])?;
            tx.execute("DELETE FROM alarm_triggers", [])?;
        }
        for name in gone {
            if let Some(id) = name.strip_prefix("Alarm/") {
                tx.execute(
                    "DELETE FROM reminder_alarms WHERE alarm_id = ?1",
                    params![id],
                )?;
            } else if let Some(id) = name.strip_prefix("AlarmTrigger/") {
                tx.execute(
                    "DELETE FROM alarm_triggers WHERE trigger_id = ?1",
                    params![id],
                )?;
            }
        }
        for a in alarms {
            tx.execute(
                "INSERT INTO reminder_alarms (alarm_id, reminder_guid, trigger_id, acknowledged_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(alarm_id) DO UPDATE SET reminder_guid = excluded.reminder_guid,
                   trigger_id = excluded.trigger_id, acknowledged_at = excluded.acknowledged_at",
                params![a.id, a.reminder_guid, a.trigger_id, a.acknowledged_at],
            )?;
        }
        for t in triggers {
            tx.execute(
                "INSERT INTO alarm_triggers (trigger_id, fires_at) VALUES (?1, ?2)
                 ON CONFLICT(trigger_id) DO UPDATE SET fires_at = excluded.fires_at",
                params![t.id, t.fires_at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Alerts whose moment has come and that have not rung, oldest first. Only for reminders
    /// still outstanding: one ticked off anywhere says nothing more.
    pub fn due_alarms(&self, now: i64) -> Result<Vec<DueAlarm>> {
        let sql = format!(
            "SELECT p.key, p.reminder_guid, r.list_guid, r.title, r.notes, l.title, p.fires_at
               FROM ({PENDING}) p
               JOIN reminders r ON r.guid = p.reminder_guid
               LEFT JOIN reminder_lists l ON l.guid = r.list_guid
              WHERE r.completed_at IS NULL AND p.fires_at <= ?1
                AND NOT EXISTS (SELECT 1 FROM alarm_fired f
                                 WHERE f.alarm_id = p.key AND f.fires_at = p.fires_at)
              ORDER BY p.fires_at, p.key"
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params![now], |row| {
            Ok(DueAlarm {
                key: row.get(0)?,
                reminder_guid: row.get(1)?,
                list_guid: row.get(2)?,
                title: row.get(3)?,
                notes: row.get(4)?,
                list_title: row.get(5)?,
                fires_at: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The next moment any alert will ring after `now`, if one is set.
    pub fn next_alarm_after(&self, now: i64) -> Result<Option<i64>> {
        let sql = format!(
            "SELECT MIN(p.fires_at)
               FROM ({PENDING}) p
               JOIN reminders r ON r.guid = p.reminder_guid
              WHERE r.completed_at IS NULL AND p.fires_at > ?1
                AND NOT EXISTS (SELECT 1 FROM alarm_fired f
                                 WHERE f.alarm_id = p.key AND f.fires_at = p.fires_at)"
        );
        Ok(self
            .conn()
            .query_row(&sql, params![now], |row| row.get(0))?)
    }

    /// Remember that an alert has rung, so it never rings again. A snooze, once rung, is done.
    /// Whether this alert, at this moment, has already had its one chance to ring.
    pub fn alarm_already_fired(&self, key: &str, fires_at: i64) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM alarm_fired WHERE alarm_id = ?1 AND fires_at = ?2)",
            params![key, fires_at],
            |r| r.get::<_, i64>(0),
        )? != 0)
    }

    pub fn mark_alarm_fired(&mut self, key: &str, fires_at: i64, now: i64) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO alarm_fired (alarm_id, fires_at, fired_at) VALUES (?1, ?2, ?3)",
            params![key, fires_at, now],
        )?;
        if let Some(reminder) = key.strip_prefix("snooze:") {
            self.conn().execute(
                "DELETE FROM alarm_snoozes WHERE reminder_guid = ?1 AND fires_at = ?2",
                params![reminder, fires_at],
            )?;
        }
        Ok(())
    }

    /// Ring about a reminder again at `until`, on this device. A newer snooze replaces one.
    pub fn snooze_reminder(&mut self, reminder_guid: &str, until: i64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO alarm_snoozes (reminder_guid, fires_at) VALUES (?1, ?2)
             ON CONFLICT(reminder_guid) DO UPDATE SET fires_at = excluded.fires_at",
            params![reminder_guid, until],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reminders::{Reminder, ReminderList};

    fn library() -> Library {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(&dir.keep()).unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "L".into(),
            title: "Home".into(),
            enabled: true,
            ..Default::default()
        }])
        .unwrap();
        lib
    }

    fn reminder(guid: &str, done: bool) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: "L".into(),
            title: format!("Do {guid}"),
            completed_at: done.then_some(1),
            ..Default::default()
        }
    }

    fn alarm(id: &str, reminder: &str, trigger: &str) -> Alarm {
        Alarm {
            id: id.into(),
            reminder_guid: reminder.into(),
            trigger_id: trigger.into(),
            acknowledged_at: None,
        }
    }

    fn trigger(id: &str, at: i64) -> AlarmTrigger {
        AlarmTrigger {
            id: id.into(),
            fires_at: at,
        }
    }

    #[test]
    fn an_alarm_rings_once_at_its_moment() {
        let mut lib = library();
        lib.put_reminders(&[reminder("R", false)]).unwrap();
        lib.put_alarms(&[alarm("A", "R", "T")], &[trigger("T", 1_000)], &[], true)
            .unwrap();

        assert!(
            lib.due_alarms(999).unwrap().is_empty(),
            "not before its moment"
        );
        assert_eq!(lib.next_alarm_after(0).unwrap(), Some(1_000));

        let due = lib.due_alarms(1_000).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].title, "Do R");
        assert_eq!(due[0].list_title.as_deref(), Some("Home"));
        lib.mark_alarm_fired(&due[0].key, due[0].fires_at, 1_000)
            .unwrap();

        assert!(lib.due_alarms(5_000).unwrap().is_empty(), "and never again");
        assert_eq!(lib.next_alarm_after(0).unwrap(), None);
        // The same alarm coming back unchanged in the next pass does not ring it again.
        lib.put_alarms(&[alarm("A", "R", "T")], &[trigger("T", 1_000)], &[], true)
            .unwrap();
        assert!(lib.due_alarms(5_000).unwrap().is_empty());
    }

    #[test]
    fn an_alarm_moved_to_a_new_time_rings_at_that_time() {
        let mut lib = library();
        lib.put_reminders(&[reminder("R", false)]).unwrap();
        lib.put_alarms(&[alarm("A", "R", "T")], &[trigger("T", 1_000)], &[], true)
            .unwrap();
        lib.mark_alarm_fired("A", 1_000, 1_000).unwrap();
        // A recurring reminder's alarm, advanced by the phone to tomorrow.
        lib.put_alarms(&[], &[trigger("T", 87_400)], &[], false)
            .unwrap();
        assert_eq!(lib.next_alarm_after(1_000).unwrap(), Some(87_400));
        assert_eq!(lib.due_alarms(87_400).unwrap().len(), 1);
    }

    #[test]
    fn nothing_rings_for_a_reminder_that_is_done_seen_elsewhere_or_gone() {
        let mut lib = library();
        lib.put_reminders(&[
            reminder("Done", true),
            reminder("Seen", false),
            reminder("Gone", false),
        ])
        .unwrap();
        let mut seen = alarm("A2", "Seen", "T2");
        seen.acknowledged_at = Some(1_010);
        lib.put_alarms(
            &[alarm("A1", "Done", "T1"), seen, alarm("A3", "Gone", "T3")],
            &[
                trigger("T1", 1_000),
                trigger("T2", 1_000),
                trigger("T3", 1_000),
            ],
            &[],
            true,
        )
        .unwrap();
        lib.put_alarms(&[], &[], &["Alarm/A3".into()], false)
            .unwrap();
        assert!(lib.due_alarms(2_000).unwrap().is_empty());
        assert_eq!(lib.next_alarm_after(0).unwrap(), None);
    }

    #[test]
    fn an_acknowledgement_from_before_the_moment_does_not_silence_it() {
        let mut lib = library();
        lib.put_reminders(&[reminder("R", false)]).unwrap();
        let mut a = alarm("A", "R", "T");
        // Seen yesterday, for yesterday's occurrence; today's is still to ring.
        a.acknowledged_at = Some(500);
        lib.put_alarms(&[a], &[trigger("T", 1_000)], &[], true)
            .unwrap();
        assert_eq!(lib.due_alarms(1_000).unwrap().len(), 1);
    }

    /// "Study", every evening: the phone moved it to the 30th and left its trigger on the
    /// 27th, which was seen on the phone that day.
    #[test]
    fn a_repeating_reminder_rings_at_its_current_occurrence() {
        let mut lib = library();
        let mut study = reminder("R", false);
        study.due_at = Some(3 * 86_400);
        study.recur_frequency = Some(1);
        study.recur_interval = Some(1);
        lib.put_reminders(&[study]).unwrap();
        let mut a = alarm("A", "R", "T");
        a.acknowledged_at = Some(4_000);
        lib.put_alarms(&[a], &[trigger("T", 0)], &[], true).unwrap();
        assert!(lib.due_alarms(3 * 86_400 - 1).unwrap().is_empty());
        assert_eq!(lib.next_alarm_after(10_000).unwrap(), Some(3 * 86_400));
        let due = lib.due_alarms(3 * 86_400).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].fires_at, 3 * 86_400);
    }

    #[test]
    fn a_snooze_rings_once_at_its_own_time() {
        let mut lib = library();
        lib.put_reminders(&[reminder("R", false)]).unwrap();
        lib.snooze_reminder("R", 4_600).unwrap();
        assert_eq!(lib.next_alarm_after(1_000).unwrap(), Some(4_600));
        let due = lib.due_alarms(4_600).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].key, "snooze:R");
        lib.mark_alarm_fired(&due[0].key, due[0].fires_at, 4_600)
            .unwrap();
        assert!(lib.due_alarms(9_000).unwrap().is_empty());
        // And snoozing again later rings again.
        lib.snooze_reminder("R", 8_200).unwrap();
        assert_eq!(lib.due_alarms(9_000).unwrap().len(), 1);
    }

    /// A calendar alert, keyed by event and lead time, rings once at its moment — and again
    /// only if the event moves and its moment with it.
    #[test]
    fn an_event_alert_is_recorded_once_per_moment() {
        let mut lib = Library::in_memory().unwrap();
        assert!(!lib.alarm_already_fired("event:E:15", 1_000).unwrap());
        lib.mark_alarm_fired("event:E:15", 1_000, 1_000).unwrap();
        assert!(lib.alarm_already_fired("event:E:15", 1_000).unwrap());
        assert!(
            !lib.alarm_already_fired("event:E:15", 4_600).unwrap(),
            "moved, it rings at its new time"
        );
    }
}
