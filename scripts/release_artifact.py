"""Bind release archives to one successful workflow attempt and immutable artifact ID."""
import json
import subprocess
import tempfile
import zipfile


def get_attempt(run_id, repository, attempt, run_command):
    build = json.loads(run_command(
        'gh', 'api', f'repos/{repository}/actions/runs/{run_id}/attempts/{attempt}'))
    if (build['id'] != int(run_id) or build['run_attempt'] != attempt or build['status'] != 'completed'
            or build['conclusion'] != 'success'):
        raise ValueError('selected build attempt is not successful')
    return build


def assert_current_attempt(run_id, repository, attempt, run_command):
    current = json.loads(run_command('gh', 'api', f'repos/{repository}/actions/runs/{run_id}'))
    if current['run_attempt'] != attempt:
        raise ValueError('candidate build was rerun; create a fresh candidate for that attempt')


def download_release(run_id, repository, attempt, dist, run_command, artifact_id=None):
    # GitHub lists artifacts by run, not attempt. The trusted uploader includes
    # run_attempt in the name; download only the selected immutable artifact ID.
    pages = json.loads(run_command('gh', 'api', f'repos/{repository}/actions/runs/{run_id}/artifacts',
                                   '--paginate', '--slurp'))
    artifacts = [artifact for page in pages for artifact in page['artifacts']
                 if artifact['name'] == f'fractal-release-attempt-{attempt}'
                 and not artifact['expired']]
    if len(artifacts) != 1 or type(artifacts[0]['id']) is not int or artifacts[0]['id'] <= 0:
        raise ValueError('build attempt must have exactly one unexpired release artifact')
    selected_id = artifacts[0]['id']
    if artifact_id is not None and selected_id != artifact_id:
        raise ValueError('candidate artifact ID differs from the successful build attempt')
    with tempfile.TemporaryFile() as archive:
        subprocess.run(['gh', 'api', f'repos/{repository}/actions/artifacts/{selected_id}/zip'],
                       check=True, stdout=archive)
        archive.seek(0)
        try:
            with zipfile.ZipFile(archive) as bundle:
                bundle.extractall(dist)
        except zipfile.BadZipFile as error:
            raise ValueError('build artifact is not a valid ZIP archive') from error
    return selected_id
