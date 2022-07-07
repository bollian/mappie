from setuptools import setup
import os

# ugly hack to make requirements.txt and setup.py consistent
lib_folder = os.path.dirname(os.path.realpath(__file__))
requirement_path = lib_folder + '/requirements.txt'
install_requires = []
if os.path.isfile(requirement_path):
    with open(requirement_path) as f:
        install_requires = f.read().splitlines()

setup(
    name='mappie',
    version='0.1',
    url='https://github.com/bollian/mappie',
    license='MPL2',
    packages=['mappie'],
    install_requires=install_requires
)
