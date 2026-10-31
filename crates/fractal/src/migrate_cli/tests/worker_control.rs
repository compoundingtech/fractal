use super::*;

fn control_directory(unit: &Unit) -> PathBuf {
    let mut filename = unit.file.file_name().unwrap().to_os_string();
    filename.push(".d");
    unit.file.parent().unwrap().parent().unwrap().join("user.control").join(filename)
}

fn rename_unit(unit: &mut Unit, name: &str) -> Result<()> {
    let filename = if cfg!(target_os = "macos") { format!("{name}.plist") } else { name.into() };
    let file = unit.file.parent().unwrap().join(filename);
    let old_dropins = dropins(&unit.file);
    fs::rename(&unit.file, &file)?;
    if old_dropins.exists() { fs::rename(old_dropins, dropins(&file))?; }
    unit.file = file;
    unit.name = name.into();
    Ok(())
}

#[tokio::test]
async fn smalltalk_replication_dropins_stop_and_retire_with_the_worker() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let legacy = fixture.manager.services.legacy.iter_mut().find(|unit| unit.name.contains("replication")).unwrap();
    rename_unit(legacy, "smalltalk-replication.service")?;
    let old = legacy.clone();
    let target = fixture.manager.services.target.iter_mut().find(|unit| unit.name.contains("replication")).unwrap();
    rename_unit(target, "fractal-replication.service")?;
    let new = target.clone();
    let inode = fs::metadata(dropins(&old.file))?.ino();
    fixture.run(false, false).await?;
    assert_eq!(fs::metadata(dropins(&new.file))?.ino(), inode);
    assert!(!dropins(&old.file).exists());
    assert!(!old.file.exists());
    assert!(fixture.manager.events.contains(&format!("stop:{}", old.name)));
    assert!(fixture.manager.events.contains(&format!("enable:{}:false", old.name)));
    assert!(fixture.manager.events.contains(&format!("start:{}", new.name)));
    #[cfg(target_os = "linux")]
    {
        let journal: Journal = serde_json::from_slice(&fs::read(&fixture.layout.journal)?)?;
        validate_journal(&fixture.layout, &journal, &fixture.manager.services)?;
    }
    Ok(())
}

#[tokio::test]
async fn user_control_dropins_are_journaled_preserved_and_stable_after_exchange() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let old = control_directory(&fixture.manager.services.legacy[0]);
    let new = control_directory(&fixture.manager.services.target[0]);
    fs::create_dir_all(&old)?;
    fs::write(old.join("50-MemorySwapMax.conf"), b"[Service]\nMemorySwapMax=0\n")?;
    let inode = fs::metadata(&old)?.ino();
    let before = fixture.snapshot()?;
    fixture.run(true, false).await?;
    assert_eq!(fixture.snapshot()?, before);
    fixture.run(false, false).await?;
    assert_eq!(fs::metadata(&new)?.ino(), inode);
    assert_eq!(fs::read(new.join("50-MemorySwapMax.conf"))?, b"[Service]\nMemorySwapMax=0\n");
    assert!(!old.exists());
    let journal: Journal = serde_json::from_slice(&fs::read(&fixture.layout.journal)?)?;
    assert!(journal.pairs.contains(&(old, new)));
    validate_journal(&fixture.layout, &journal, &fixture.manager.services)?;
    fixture.run(true, false).await?;
    Ok(())
}

#[tokio::test]
async fn user_control_source_symlinks_and_target_collisions_refuse_before_mutation() -> Result<()> {
    for symlink_source in [false, true] {
        let mut fixture = Fixture::new()?;
        let old = control_directory(&fixture.manager.services.legacy[0]);
        let new = control_directory(&fixture.manager.services.target[0]);
        fs::create_dir_all(old.parent().unwrap())?;
        if symlink_source {
            let backing = fixture.root.path().join("control-backing");
            fs::create_dir(&backing)?;
            symlink(backing, &old)?;
        } else {
            fs::create_dir(&old)?;
            fs::write(old.join("50-MemorySwapMax.conf"), b"[Service]\nMemorySwapMax=0\n")?;
            fs::create_dir(&new)?;
            fs::write(new.join("50-MemorySwapMax.conf"), b"[Service]\nMemorySwapMax=infinity\n")?;
        }
        let before = fixture.snapshot()?;
        let error = fixture.run(false, false).await.unwrap_err();
        if symlink_source {
            assert_eq!(error.to_string(), format!("{} must be an ordinary systemd control drop-in directory", old.display()));
        } else {
            assert!(error.to_string().starts_with(&format!("destination {} is non-empty; refusing to overwrite it", new.display())));
        }
        assert_eq!(fixture.snapshot()?, before);
        assert!(!fixture.manager.changed_services());
        assert!(!fixture.layout.journal.exists());
    }
    Ok(())
}

#[tokio::test]
async fn user_control_dropins_return_unchanged_when_the_api_probe_fails() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let old = control_directory(&fixture.manager.services.legacy[0]);
    fs::create_dir_all(&old)?;
    fs::write(old.join("50-MemorySwapMax.conf"), b"[Service]\nMemorySwapMax=0\n")?;
    let before = fixture.snapshot()?;
    fixture.manager.probe_failure = true;
    let error = fixture.run(false, false).await.unwrap_err();
    assert!(format!("{error:#}").contains("legacy paths and services restored"));
    assert_eq!(fixture.snapshot()?, before);
    Ok(())
}

#[tokio::test]
async fn retained_journal_refuses_new_control_pairs_without_reinterpreting_progress() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.run(false, false).await?;
    let target_control = control_directory(&fixture.manager.services.target[0]);
    fs::create_dir_all(&target_control)?;
    fs::write(target_control.join("50-MemorySwapMax.conf"), b"[Service]\nMemorySwapMax=0\n")?;
    fixture.manager.events.clear();
    let before = fixture.snapshot()?;
    let error = fixture.run(true, false).await.unwrap_err();
    assert_eq!(error.to_string(), "journal paths do not match this host");
    assert_eq!(fixture.snapshot()?, before);
    assert!(!fixture.manager.changed_services());
    Ok(())
}
